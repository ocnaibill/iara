//! O serviço: dono do perfil ativo, supervisor do motor de áudio e do autosave. Orientado a eventos: espera por
//! comandos e eventos do motor num único canal, acordando só para os prazos (autosave, fechamento de gesto, reconexão).
//! O tempo entra por parâmetro (`handle`, `tick`) para testar sem dormir; `run` liga ao relógio real.

use crate::apps::{self, AppView, Overrides};
use crate::tracker::{Action, EditKind, EditTracker};
use iara_audio::{AppReport, ApplyReport, Engine, EngineError, Event, EventSink, RouteTarget};
use iara_core::edit::{self, EditClass, EditCommand};
use iara_core::topology::{plan, Plan};
use iara_core::{initial_profile, Profile};
use iara_store::{GlobalConfig, RevisionReason, Store, StoreError};
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Esperas entre tentativas de reconectar ao PipeWire (a última se repete).
const BACKOFF: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(10),
];

pub trait Backend {
    fn apply(&mut self, plan: Plan) -> Result<ApplyReport, EngineError>;
    /// Para onde cada aplicativo (chave de identidade) deve ir; o resultado volta como `Event::Apps`.
    fn set_routes(&mut self, routes: HashMap<String, RouteTarget>) -> Result<(), EngineError>;
}

impl Backend for Engine {
    fn apply(&mut self, plan: Plan) -> Result<ApplyReport, EngineError> {
        Engine::apply(self, plan)
    }

    fn set_routes(&mut self, routes: HashMap<String, RouteTarget>) -> Result<(), EngineError> {
        Engine::set_routes(self, routes)
    }
}

/// Resposta a um comando: o serviço responde por este canal (o IPC espera nele com prazo).
pub type Reply<T> = mpsc::Sender<T>;

/// Retrato completo do estado do serviço para uma interface: versão que só cresce, perfil ativo e status.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub serial: u64,
    pub profile: Profile,
    pub status: Status,
    /// Aplicativos para a interface: tocando agora (com estado real) e com regra salva mas em silêncio.
    pub apps: Vec<AppView>,
}

/// Escolha temporária (só nesta sessão) para um aplicativo; some quando ele para de tocar, ao trocar de perfil ou ao
/// reaplicar a regra (spec 8.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionChoice {
    Channel(String),
    Unassigned,
    /// Reaplica a regra do perfil (apaga a escolha temporária).
    Clear,
}

pub enum Command {
    /// Edição granular do perfil ativo (a forma normal de a interface mudar o estado): responde com a nova versão ou o erro.
    Edit {
        cmd: EditCommand,
        reply: Option<Reply<Result<u64, String>>>,
    },
    /// Retrato atual do estado.
    GetState {
        reply: Reply<Snapshot>,
    },
    /// Escolha temporária de canal para um aplicativo (chave de identidade), sem alterar o perfil.
    SessionChoice {
        key: String,
        choice: SessionChoice,
        reply: Option<Reply<Result<u64, String>>>,
    },
    /// Substituição do perfil inteiro (importação, restauração de revisão, reset) e o tipo de edição para o histórico.
    SetProfile {
        profile: Box<Profile>,
        kind: EditKind,
    },
    /// Grava e fecha o gesto agora (concluir gesto, trocar de perfil).
    Flush,
    Shutdown,
}

pub enum Msg {
    Command(Command),
    Engine(Event),
}

pub type Connector<B> = Box<dyn FnMut(EventSink) -> Result<B, EngineError>>;

#[derive(Debug)]
pub enum ServiceError {
    Store(StoreError),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<StoreError> for ServiceError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// O que a interface precisa mostrar (spec 6.2, 8.10): conexão, falha de gravação, último relatório, dispositivos ausentes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Status {
    pub connected: bool,
    pub persist_error: Option<String>,
    pub last_report: Option<ApplyReport>,
    pub absent_devices: Vec<String>,
    pub reconnect_attempts: u32,
    /// Último relatório de aplicativos do motor (fluxos agrupados por aplicativo).
    pub apps: Vec<AppReport>,
}

pub struct Service<B: Backend> {
    store: Store,
    profile: Profile,
    tracker: EditTracker,
    backend: Option<B>,
    connect: Connector<B>,
    sink: EventSink,
    retry: Option<(Instant, u32)>,
    status: Status,
    stopped: bool,
    serial: u64,
    notifier: Option<Box<dyn Fn(u64)>>,
    overrides: Overrides,
    last_routes: Option<HashMap<String, RouteTarget>>,
}

/// Perfil ativo da configuração; sem nenhum, usa o primeiro existente ou cria o padrão. Nunca sobrescreve um perfil
/// ilegível: se o arquivo existe mas não carrega (nem pela cópia), devolve o erro.
fn load_or_create_profile(store: &Store) -> Result<Profile, ServiceError> {
    let (mut config, _) = store.load_config()?;
    let candidates = config
        .active_profile
        .clone()
        .into_iter()
        .chain(store.list_profiles()?)
        .collect::<Vec<_>>();
    for id in candidates {
        match store.load_profile(&id) {
            Ok((p, _)) => {
                if config.active_profile.as_deref() != Some(&p.id) {
                    config.active_profile = Some(p.id.clone());
                    store.save_config(&config)?;
                }
                return Ok(p);
            }
            Err(StoreError::NotFound(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    let p = initial_profile();
    store.save_profile(&p)?;
    store.save_config(&GlobalConfig {
        active_profile: Some(p.id.clone()),
        ..config
    })?;
    Ok(p)
}

impl<B: Backend> Service<B> {
    pub fn new(store: Store, connect: Connector<B>, sink: EventSink) -> Result<Self, ServiceError> {
        let profile = load_or_create_profile(&store)?;
        Ok(Self {
            store,
            profile,
            tracker: EditTracker::default(),
            backend: None,
            connect,
            sink,
            retry: None,
            status: Status::default(),
            stopped: false,
            serial: 1,
            notifier: None,
            overrides: Overrides::new(),
            last_routes: None,
        })
    }

    /// Registra quem é avisado a cada mudança de estado visível (perfil ou status), com a nova versão.
    pub fn set_notifier(&mut self, notifier: Box<dyn Fn(u64)>) {
        self.notifier = Some(notifier);
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            serial: self.serial,
            profile: self.profile.clone(),
            status: self.status.clone(),
            apps: apps::views(&self.profile, &self.overrides, &self.status.apps),
        }
    }

    /// Manda o plano de rotas ao motor quando ele mudou (ou sempre, com `force`, ao reconectar).
    fn sync_routes(&mut self, force: bool, now: Instant) {
        let plan = apps::route_plan(&self.profile, &self.overrides, &self.status.apps);
        if !force && self.last_routes.as_ref() == Some(&plan) {
            return;
        }
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        match backend.set_routes(plan.clone()) {
            Ok(()) => self.last_routes = Some(plan),
            Err(EngineError::Disconnected) => self.drop_backend(now),
            Err(e) => eprintln!("iara: falha ao enviar as rotas: {e}"),
        }
    }

    fn changed(&mut self) {
        self.serial += 1;
        if let Some(n) = &self.notifier {
            n(self.serial);
        }
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    pub fn start(&mut self, now: Instant) {
        self.try_connect(now);
    }

    fn try_connect(&mut self, now: Instant) {
        match (self.connect)(self.sink.clone()) {
            Ok(backend) => {
                if self.status.reconnect_attempts > 0 {
                    eprintln!("iara: reconectado ao PipeWire; reaplicando o perfil");
                }
                self.backend = Some(backend);
                self.retry = None;
                self.status.connected = true;
                self.status.reconnect_attempts = 0;
                self.apply_current(now);
                self.sync_routes(true, now);
                self.changed();
            }
            Err(e) => {
                eprintln!("iara: sem conexão com o áudio ({e}); nova tentativa em breve");
                self.schedule_retry(now);
            }
        }
    }

    fn schedule_retry(&mut self, now: Instant) {
        let attempt = self.retry.map_or(0, |(_, a)| a + 1);
        let delay = BACKOFF[(attempt as usize).min(BACKOFF.len() - 1)];
        self.retry = Some((now + delay, attempt));
        self.status.reconnect_attempts = attempt + 1;
    }

    fn drop_backend(&mut self, now: Instant) {
        eprintln!("iara: conexão com o PipeWire perdida; reconectando");
        self.backend = None; // o Engine remove os próprios objetos ao ser descartado
        self.status.connected = false;
        self.status.absent_devices.clear();
        self.status.apps.clear();
        self.last_routes = None;
        if self.retry.is_none() {
            self.retry = Some((now + BACKOFF[0], 0));
            self.status.reconnect_attempts = 1;
        }
        self.changed();
    }

    fn apply_current(&mut self, now: Instant) {
        let Ok(plan) = plan(&self.profile) else {
            return;
        };
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        match backend.apply(plan) {
            Ok(report) => {
                if !report.is_complete() {
                    eprintln!("iara: aplicação parcial, ausentes: {:?}", report.missing);
                }
                if self.status.last_report.as_ref() != Some(&report) {
                    self.status.last_report = Some(report);
                    self.changed();
                }
            }
            Err(EngineError::Disconnected) => self.drop_backend(now),
            Err(e) => eprintln!("iara: falha ao aplicar o perfil: {e}"),
        }
    }

    fn execute(&mut self, actions: Vec<Action>) {
        for action in actions {
            let result = match action {
                Action::Save => self.store.save_profile(&self.profile),
                Action::PushRevision(previous, reason) => {
                    self.store.push_revision(&previous, reason).map(|_| ())
                }
            };
            match result {
                Ok(()) => {
                    if self.status.persist_error.is_some() && self.tracker.next_deadline().is_none()
                    {
                        self.status.persist_error = None;
                        self.changed();
                    }
                }
                Err(e) => {
                    eprintln!("iara: falha ao gravar: {e}");
                    self.status.persist_error = Some(e.to_string());
                    self.changed();
                }
            }
        }
    }

    fn replace_profile(&mut self, next: Profile, kind: EditKind, now: Instant) {
        let previous = std::mem::replace(&mut self.profile, next);
        self.changed();
        self.apply_current(now);
        self.sync_routes(false, now);
        let actions = self.tracker.on_edit(&previous, kind, now);
        self.execute(actions);
    }

    pub fn handle(&mut self, msg: Msg, now: Instant) {
        match msg {
            Msg::Command(Command::SetProfile { profile, kind }) => {
                if let Err(e) = plan(&profile) {
                    eprintln!("iara: perfil recusado: {e:?}");
                    return;
                }
                self.replace_profile(*profile, kind, now);
            }
            Msg::Command(Command::Edit { cmd, reply }) => {
                let result = match edit::apply(&self.profile, &cmd) {
                    Ok((next, class)) => {
                        let kind = match class {
                            EditClass::Continuous => EditKind::Continuous,
                            EditClass::Structural => {
                                EditKind::Structural(RevisionReason::Structural)
                            }
                        };
                        self.replace_profile(next, kind, now);
                        Ok(self.serial)
                    }
                    Err(e) => Err(e.to_string()),
                };
                if let Some(r) = reply {
                    let _ = r.send(result);
                }
            }
            Msg::Command(Command::GetState { reply }) => {
                let _ = reply.send(self.snapshot());
            }
            Msg::Command(Command::SessionChoice { key, choice, reply }) => {
                let result = match choice {
                    SessionChoice::Channel(id)
                        if !self.profile.channels.iter().any(|c| c.id == id) =>
                    {
                        Err(format!("canal desconhecido: {id}"))
                    }
                    SessionChoice::Channel(id) => {
                        self.overrides.insert(key, Some(id));
                        Ok(())
                    }
                    SessionChoice::Unassigned => {
                        self.overrides.insert(key, None);
                        Ok(())
                    }
                    SessionChoice::Clear => {
                        self.overrides.remove(&key);
                        Ok(())
                    }
                };
                let result = result.map(|()| {
                    self.sync_routes(false, now);
                    self.changed();
                    self.serial
                });
                if let Some(r) = reply {
                    let _ = r.send(result);
                }
            }
            Msg::Command(Command::Flush) => {
                let actions = self.tracker.flush();
                self.execute(actions);
            }
            Msg::Command(Command::Shutdown) => {
                let actions = self.tracker.flush();
                self.execute(actions);
                self.backend = None;
                self.status.connected = false;
                self.stopped = true;
            }
            Msg::Engine(Event::Disconnected) => self.drop_backend(now),
            Msg::Engine(Event::DeviceAbsent(d)) => {
                if !self.status.absent_devices.contains(&d) {
                    self.status.absent_devices.push(d);
                    self.changed();
                }
            }
            Msg::Engine(Event::Apps(reports)) => {
                if self.status.apps != reports {
                    self.status.apps = reports;
                    if apps::prune_overrides(&mut self.overrides, &self.status.apps) {
                        eprintln!("iara: escolha temporária encerrada (aplicativo parou de tocar)");
                    }
                    self.sync_routes(false, now);
                    self.changed();
                }
            }
            Msg::Engine(Event::DeviceBack(d)) => {
                self.status.absent_devices.retain(|x| *x != d);
                self.changed();
            }
        }
    }

    pub fn tick(&mut self, now: Instant) {
        if self.retry.is_some_and(|(at, _)| now >= at) {
            self.try_connect(now);
        }
        let actions = self.tracker.on_tick(now);
        self.execute(actions);
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        [self.tracker.next_deadline(), self.retry.map(|(at, _)| at)]
            .into_iter()
            .flatten()
            .min()
    }

    /// Laço principal: dorme até o próximo prazo ou mensagem; sem prazos, espera só por mensagens.
    pub fn run(&mut self, rx: &mpsc::Receiver<Msg>) {
        self.start(Instant::now());
        while !self.stopped {
            let msg = match self.next_deadline() {
                Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(m) => self.handle(m, Instant::now()),
                Err(mpsc::RecvTimeoutError::Timeout) => self.tick(Instant::now()),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.handle(Msg::Command(Command::Shutdown), Instant::now());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::{AUTOSAVE_DELAY, GROUP_IDLE};
    use iara_core::Gain;
    use iara_store::RevisionReason;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Arc;

    #[derive(Default)]
    struct World {
        applied: RefCell<Vec<Plan>>,
        routes: RefCell<Vec<HashMap<String, RouteTarget>>>,
        connects: Cell<u32>,
        fail_connects: Cell<u32>,
        fail_apply_disconnected: Cell<bool>,
    }

    struct Fake(Rc<World>);

    impl Backend for Fake {
        fn apply(&mut self, plan: Plan) -> Result<ApplyReport, EngineError> {
            if self.0.fail_apply_disconnected.get() {
                return Err(EngineError::Disconnected);
            }
            self.0.applied.borrow_mut().push(plan);
            Ok(ApplyReport {
                observed: 1,
                missing: vec![],
                absent_devices: vec![],
            })
        }

        fn set_routes(&mut self, routes: HashMap<String, RouteTarget>) -> Result<(), EngineError> {
            if self.0.fail_apply_disconnected.get() {
                return Err(EngineError::Disconnected);
            }
            self.0.routes.borrow_mut().push(routes);
            Ok(())
        }
    }

    fn service(dir: &tempfile::TempDir) -> (Service<Fake>, Rc<World>) {
        let world = Rc::new(World::default());
        let w = world.clone();
        let connect: Connector<Fake> = Box::new(move |_sink| {
            w.connects.set(w.connects.get() + 1);
            if w.fail_connects.get() > 0 {
                w.fail_connects.set(w.fail_connects.get() - 1);
                return Err(EngineError::Connect("sem daemon".into()));
            }
            Ok(Fake(w.clone()))
        });
        let store = Store::open(dir.path().join("config"), dir.path().join("state"));
        let sink: EventSink = Arc::new(|_| {});
        (Service::new(store, connect, sink).unwrap(), world)
    }

    fn set(profile: &Profile, kind: EditKind) -> Msg {
        Msg::Command(Command::SetProfile {
            profile: Box::new(profile.clone()),
            kind,
        })
    }

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn first_run_creates_and_reuses_the_default_profile() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service(&dir);
        assert_eq!(svc.profile().id, "default");
        let store = Store::open(dir.path().join("config"), dir.path().join("state"));
        assert_eq!(store.list_profiles().unwrap(), ["default"]);
        assert_eq!(
            store.load_config().unwrap().0.active_profile.as_deref(),
            Some("default")
        );
        // segunda execução: mesmo perfil, sem recriar
        let mut p = store.load_profile("default").unwrap().0;
        p.channels[0].personal.gain = Gain::from_db(-9.0).unwrap();
        store.save_profile(&p).unwrap();
        let (svc2, _) = service(&dir);
        assert_eq!(svc2.profile().channels[0].personal.gain.db(), Some(-9.0));
    }

    #[test]
    fn an_unreadable_profile_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let (_svc, _) = service(&dir);
        let file = dir.path().join("config/profiles/default.toml");
        std::fs::write(&file, "lixo [").unwrap();
        std::fs::write(
            dir.path().join("config/profiles/default.toml.bak"),
            "lixo também",
        )
        .unwrap();
        let store = Store::open(dir.path().join("config"), dir.path().join("state"));
        let connect: Connector<Fake> = Box::new(|_| Err(EngineError::Disconnected));
        assert!(Service::new(store, connect, Arc::new(|_| {})).is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "lixo [");
    }

    #[test]
    fn connect_failures_back_off_and_the_first_success_applies_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, world) = service(&dir);
        world.fail_connects.set(3);
        let t0 = Instant::now();
        svc.start(t0);
        assert!(!svc.status().connected);
        assert_eq!(svc.next_deadline(), Some(t0 + s(1)));
        svc.tick(t0 + s(1)); // 2ª tentativa falha → espera 2 s
        assert_eq!(svc.next_deadline(), Some(t0 + s(1) + s(2)));
        svc.tick(t0 + s(3)); // 3ª falha → 4 s
        assert_eq!(svc.next_deadline(), Some(t0 + s(3) + s(4)));
        assert_eq!(world.connects.get(), 3);
        svc.tick(t0 + s(7)); // 4ª tentativa conecta
        assert!(svc.status().connected && svc.next_deadline().is_none());
        assert_eq!(world.applied.borrow().len(), 1);
        assert!(svc.status().last_report.as_ref().unwrap().is_complete());
    }

    #[test]
    fn after_a_disconnect_the_service_reconnects_and_reapplies_the_current_profile() {
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, world) = service(&dir);
        let t0 = Instant::now();
        svc.start(t0);
        let mut p = svc.profile().clone();
        p.channels[1].personal.muted = true;
        svc.handle(set(&p, EditKind::Continuous), t0);
        assert_eq!(world.applied.borrow().len(), 2);

        svc.tick(t0 + s(3)); // autosave e fechamento do gesto vencem antes; só sobra o prazo de reconexão
        svc.handle(Msg::Engine(Event::Disconnected), t0 + s(5));
        assert!(!svc.status().connected);
        assert_eq!(
            svc.next_deadline().map(|d| d.duration_since(t0)),
            Some(s(6))
        );
        svc.tick(t0 + s(6));
        assert!(svc.status().connected);
        let applied = world.applied.borrow();
        assert_eq!(applied.len(), 3);
        // reaplicou o perfil ATUAL (com o mute), não o do início
        let chat = iara_core::topology::channel_node("chat");
        let branch = applied[2]
            .branches
            .iter()
            .find(|b| b.from == chat && b.to == iara_core::topology::MIX_PERSONAL)
            .unwrap();
        assert!(branch.muted);
    }

    #[test]
    fn a_disconnect_reported_by_apply_also_triggers_reconnection() {
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, world) = service(&dir);
        let t0 = Instant::now();
        svc.start(t0);
        world.fail_apply_disconnected.set(true);
        let mut p = svc.profile().clone();
        p.microphone.global_mute = true;
        svc.handle(set(&p, EditKind::Continuous), t0 + s(1));
        assert!(!svc.status().connected && svc.next_deadline().is_some());
        world.fail_apply_disconnected.set(false);
        svc.tick(t0 + s(2));
        assert!(svc.status().connected);
    }

    #[test]
    fn autosave_and_history_follow_the_timers_and_never_block_on_the_backend() {
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, _) = service(&dir);
        let store = Store::open(dir.path().join("config"), dir.path().join("state"));
        let t0 = Instant::now();
        svc.start(t0);
        let original = svc.profile().clone();
        let mut p = original.clone();
        for i in 1..=20u32 {
            p.channels[0].personal.gain = Gain::from_db(-f64::from(i)).unwrap();
            svc.handle(
                set(&p, EditKind::Continuous),
                t0 + Duration::from_millis(u64::from(i) * 10),
            );
        }
        let last = t0 + Duration::from_millis(200);
        // antes de 300 ms do último ajuste: disco ainda no estado antigo
        svc.tick(last + AUTOSAVE_DELAY - Duration::from_millis(1));
        assert_eq!(store.load_profile("default").unwrap().0, original);
        svc.tick(last + AUTOSAVE_DELAY);
        assert_eq!(store.load_profile("default").unwrap().0, p);
        assert!(
            store.list_revisions("default").unwrap().is_empty(),
            "o gesto ainda está aberto"
        );
        svc.tick(last + GROUP_IDLE);
        let revs = store.list_revisions("default").unwrap();
        assert_eq!(revs.len(), 1, "20 passos = uma revisão");
        assert_eq!(revs[0].reason, RevisionReason::Adjustment);
        assert_eq!(
            store.load_revision("default", revs[0].seq).unwrap(),
            original
        );
    }

    #[test]
    fn structural_edits_and_shutdown_flush_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, _) = service(&dir);
        let store = Store::open(dir.path().join("config"), dir.path().join("state"));
        let t0 = Instant::now();
        svc.start(t0);
        let mut p = svc.profile().clone();
        p.channels.retain(|c| c.id != "aux");
        p.chatmix.position = 0.0;
        svc.handle(
            set(&p, EditKind::Structural(RevisionReason::Structural)),
            t0,
        );
        assert_eq!(
            store.list_revisions("default").unwrap().len(),
            1,
            "revisão estrutural imediata"
        );
        // desligar antes do autosave vencer: grava mesmo assim
        svc.handle(
            Msg::Command(Command::Shutdown),
            t0 + Duration::from_millis(10),
        );
        assert!(svc.is_stopped());
        assert!(store
            .load_profile("default")
            .unwrap()
            .0
            .channels
            .iter()
            .all(|c| c.id != "aux"));
    }

    #[test]
    fn an_invalid_profile_is_refused_and_a_write_failure_is_visible() {
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, world) = service(&dir);
        let t0 = Instant::now();
        svc.start(t0);
        let mut bad = svc.profile().clone();
        bad.chatmix.position = 5.0;
        svc.handle(set(&bad, EditKind::Continuous), t0);
        assert_eq!(svc.profile().chatmix.position, 0.0);
        assert_eq!(world.applied.borrow().len(), 1, "nada foi aplicado");
        assert!(svc.next_deadline().is_none());

        // diretório de perfis vira arquivo: a gravação falha e o estado de persistência mostra isso
        let profiles = dir.path().join("config/profiles");
        std::fs::remove_dir_all(&profiles).unwrap();
        std::fs::write(&profiles, "x").unwrap();
        let mut p = svc.profile().clone();
        p.microphone.global_mute = true;
        svc.handle(set(&p, EditKind::Continuous), t0);
        svc.tick(t0 + AUTOSAVE_DELAY);
        assert!(svc.status().persist_error.is_some());
        assert!(
            svc.profile().microphone.global_mute,
            "o estado em memória segue aplicado"
        );
    }

    #[test]
    fn device_events_are_tracked_for_the_interface() {
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, _) = service(&dir);
        let t0 = Instant::now();
        svc.start(t0);
        svc.handle(Msg::Engine(Event::DeviceAbsent("fone".into())), t0);
        svc.handle(Msg::Engine(Event::DeviceAbsent("fone".into())), t0);
        assert_eq!(svc.status().absent_devices, ["fone"]);
        svc.handle(Msg::Engine(Event::DeviceBack("fone".into())), t0);
        assert!(svc.status().absent_devices.is_empty());
    }

    fn edit(cmd: EditCommand) -> (Msg, mpsc::Receiver<Result<u64, String>>) {
        let (tx, rx) = mpsc::channel();
        (
            Msg::Command(Command::Edit {
                cmd,
                reply: Some(tx),
            }),
            rx,
        )
    }

    #[test]
    fn edit_commands_change_the_profile_apply_to_the_backend_and_report_the_new_serial() {
        use iara_core::edit::SendKind;
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, world) = service(&dir);
        let t0 = Instant::now();
        svc.start(t0);
        let before = svc.snapshot().serial;
        let (msg, rx) = edit(EditCommand::SetChannelGain {
            channel: "game".into(),
            send: SendKind::Personal,
            gain: Gain::from_db(-6.0).unwrap(),
        });
        svc.handle(msg, t0);
        let serial = rx.try_recv().unwrap().unwrap();
        assert!(serial > before);
        assert_eq!(svc.snapshot().serial, serial);
        assert_eq!(svc.profile().channels[0].personal.gain.db(), Some(-6.0));
        assert_eq!(world.applied.borrow().len(), 2, "aplicado ao backend");
        // estrutural: revisão própria imediata, via o mesmo caminho
        let (msg, rx) = edit(EditCommand::AddChannel {
            id: "musica".into(),
            name: "Música".into(),
        });
        svc.handle(msg, t0);
        assert!(rx.try_recv().unwrap().is_ok());
        let store = Store::open(dir.path().join("config"), dir.path().join("state"));
        assert!(store
            .list_revisions("default")
            .unwrap()
            .iter()
            .any(|r| r.reason == RevisionReason::Structural));
    }

    #[test]
    fn a_rejected_edit_replies_with_the_error_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, world) = service(&dir);
        let t0 = Instant::now();
        svc.start(t0);
        let snap = svc.snapshot();
        let (msg, rx) = edit(EditCommand::SetChatMixPosition(9.0));
        svc.handle(msg, t0);
        assert!(rx.try_recv().unwrap().unwrap_err().contains("ChatMix"));
        let (msg, rx) = edit(EditCommand::RemoveChannel {
            channel: "fantasma".into(),
            destination: None,
        });
        svc.handle(msg, t0);
        assert!(rx.try_recv().unwrap().unwrap_err().contains("fantasma"));
        assert_eq!(svc.snapshot(), snap, "perfil, status e versão intactos");
        assert_eq!(world.applied.borrow().len(), 1);
        assert!(svc.next_deadline().is_none(), "nem autosave foi agendado");
    }

    #[test]
    fn get_state_returns_the_snapshot_and_the_notifier_sees_every_visible_change() {
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, _) = service(&dir);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        svc.set_notifier(Box::new(move |n| log.borrow_mut().push(n)));
        let t0 = Instant::now();
        svc.start(t0); // conecta: status muda
        let (tx, rx) = mpsc::channel();
        svc.handle(Msg::Command(Command::GetState { reply: tx }), t0);
        let snap = rx.try_recv().unwrap();
        assert!(snap.status.connected);
        assert_eq!(snap.serial, svc.snapshot().serial);

        svc.handle(Msg::Engine(Event::DeviceAbsent("fone".into())), t0);
        let (msg, _rx) = edit(EditCommand::SetMicGlobalMute(true));
        svc.handle(msg, t0);
        svc.handle(Msg::Engine(Event::Disconnected), t0 + s(1));
        let seen = seen.borrow();
        assert!(seen.len() >= 4);
        assert!(
            seen.windows(2).all(|w| w[0] < w[1]),
            "versões estritamente crescentes: {seen:?}"
        );
        assert_eq!(*seen.last().unwrap(), svc.snapshot().serial);
    }

    fn zen_report(state: iara_audio::AppRouteState) -> AppReport {
        use iara_core::apps::AppIdentity;
        AppReport {
            identity: AppIdentity {
                binary: Some("zen".into()),
                name: Some("Zen".into()),
                ..Default::default()
            },
            state,
            streams: vec![],
        }
    }

    fn assign(binary: &str, channel: Option<&str>) -> EditCommand {
        EditCommand::AssignApp {
            matcher: iara_core::apps::AppMatcher {
                binary: Some(binary.into()),
                ..Default::default()
            },
            channel: channel.map(Into::into),
        }
    }

    fn last_route(world: &World, key: &str) -> Option<RouteTarget> {
        world
            .routes
            .borrow()
            .last()
            .and_then(|m| m.get(key).cloned())
    }

    #[test]
    fn routes_follow_rules_and_sessions_and_are_only_resent_when_they_change() {
        use iara_audio::AppRouteState;
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, world) = service(&dir);
        let t0 = Instant::now();
        svc.start(t0);
        let sent = |w: &World| w.routes.borrow().len();
        assert_eq!(sent(&world), 1, "ao conectar, o plano (vazio) é enviado");

        // o motor reporta o Zen sem regra: vai para Não atribuídos (padrão do sistema)
        svc.handle(
            Msg::Engine(Event::Apps(vec![zen_report(AppRouteState::Unmanaged)])),
            t0,
        );
        assert_eq!(last_route(&world, "bin:zen"), Some(RouteTarget::Default));
        let n = sent(&world);
        // o mesmo relatório de novo não reenvia
        svc.handle(
            Msg::Engine(Event::Apps(vec![zen_report(AppRouteState::Unmanaged)])),
            t0,
        );
        assert_eq!(sent(&world), n);

        // regra salva: o aplicativo passa ao canal
        let (msg, rx) = edit(assign("zen", Some("media")));
        svc.handle(msg, t0);
        assert!(rx.try_recv().unwrap().is_ok());
        assert_eq!(
            last_route(&world, "bin:zen"),
            Some(RouteTarget::Node("iara.ch.media".into()))
        );
        assert_eq!(svc.profile().rules.len(), 1);

        // escolha só desta sessão vence a regra e NÃO altera o perfil
        let (tx, rx) = mpsc::channel();
        svc.handle(
            Msg::Command(Command::SessionChoice {
                key: "bin:zen".into(),
                choice: SessionChoice::Channel("game".into()),
                reply: Some(tx),
            }),
            t0,
        );
        assert!(rx.try_recv().unwrap().is_ok());
        assert_eq!(
            last_route(&world, "bin:zen"),
            Some(RouteTarget::Node("iara.ch.game".into()))
        );
        assert_eq!(
            svc.profile().rules[0].channel,
            "media",
            "a regra salva não mudou"
        );
        let snap = svc.snapshot();
        assert_eq!(
            (snap.apps[0].channel.as_deref(), snap.apps[0].source),
            (Some("game"), iara_core::apps::Source::SessionOverride)
        );

        // "reaplicar a regra" apaga a escolha temporária
        let (tx, _rx) = mpsc::channel();
        svc.handle(
            Msg::Command(Command::SessionChoice {
                key: "bin:zen".into(),
                choice: SessionChoice::Clear,
                reply: Some(tx),
            }),
            t0,
        );
        assert_eq!(
            last_route(&world, "bin:zen"),
            Some(RouteTarget::Node("iara.ch.media".into()))
        );
    }

    #[test]
    fn a_session_choice_ends_when_the_app_stops_and_unknown_channels_are_refused() {
        use iara_audio::AppRouteState;
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, world) = service(&dir);
        let t0 = Instant::now();
        svc.start(t0);
        svc.handle(
            Msg::Engine(Event::Apps(vec![zen_report(AppRouteState::Applied)])),
            t0,
        );
        let (tx, rx) = mpsc::channel();
        svc.handle(
            Msg::Command(Command::SessionChoice {
                key: "bin:zen".into(),
                choice: SessionChoice::Channel("fantasma".into()),
                reply: Some(tx),
            }),
            t0,
        );
        assert!(rx.try_recv().unwrap().unwrap_err().contains("fantasma"));
        let (tx, _rx) = mpsc::channel();
        svc.handle(
            Msg::Command(Command::SessionChoice {
                key: "bin:zen".into(),
                choice: SessionChoice::Unassigned,
                reply: Some(tx),
            }),
            t0,
        );
        assert_eq!(svc.snapshot().apps[0].channel, None);
        // o aplicativo sai (some do relatório): a escolha termina
        svc.handle(Msg::Engine(Event::Apps(vec![])), t0);
        svc.handle(
            Msg::Engine(Event::Apps(vec![zen_report(AppRouteState::Applied)])),
            t0,
        );
        assert_eq!(last_route(&world, "bin:zen"), Some(RouteTarget::Default));
        let snap = svc.snapshot();
        assert_eq!(snap.apps[0].source, iara_core::apps::Source::Default);
    }

    #[test]
    fn silent_rules_show_as_waiting_and_a_reconnect_resends_the_routes() {
        use iara_audio::AppRouteState;
        let dir = tempfile::tempdir().unwrap();
        let (mut svc, world) = service(&dir);
        let t0 = Instant::now();
        svc.start(t0);
        let (msg, _rx) = edit(assign("discord", Some("chat")));
        svc.handle(msg, t0);
        let snap = svc.snapshot();
        assert_eq!(snap.apps.len(), 1);
        assert_eq!(
            (snap.apps[0].state, snap.apps[0].channel.as_deref()),
            (crate::apps::AppState::Waiting, Some("chat"))
        );

        svc.handle(
            Msg::Engine(Event::Apps(vec![zen_report(AppRouteState::Applied)])),
            t0,
        );
        assert_eq!(svc.status().apps.len(), 1);
        let before = world.routes.borrow().len();
        svc.handle(Msg::Engine(Event::Disconnected), t0 + s(1));
        assert!(
            svc.status().apps.is_empty(),
            "inventário antigo some junto com a conexão"
        );
        svc.tick(t0 + s(2));
        assert!(svc.status().connected);
        assert!(
            world.routes.borrow().len() > before,
            "após reconectar, o plano é reenviado"
        );
    }
}

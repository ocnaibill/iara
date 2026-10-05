//! Persistência do Iara: perfis e configuração em TOML versionado, gravação atômica com cópia válida anterior,
//! histórico de revisões (50 por perfil, FIFO), duplicação, importação e exportação (spec 7, 8.10, 8.14).
//!
//! Sem agrupamento temporal aqui: o autosave (300 ms) e o agrupamento de gestos para o histórico (2 s) são do serviço,
//! que chama `save_profile` e `push_revision` nos momentos certos.

mod dto;

pub use dto::GlobalConfig;
use dto::{profile_from_dto, profile_to_dto, ProfileDto, SCHEMA_VERSION};
use iara_core::{is_valid_id, topology::plan, Profile, HISTORY_LIMIT};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    Parse(String),
    /// Arquivo de uma versão de schema mais nova que a suportada: não é lido nem sobrescrito.
    Schema {
        found: u32,
        supported: u32,
    },
    Invalid(String),
    NotFound(String),
    Exists(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "erro de E/S: {e}"),
            Self::Parse(m) => write!(f, "arquivo ilegível: {m}"),
            Self::Schema { found, supported } => write!(
                f,
                "schema {found} é mais novo que o suportado ({supported}); atualize o Iara"
            ),
            Self::Invalid(m) => write!(f, "inválido: {m}"),
            Self::NotFound(m) => write!(f, "não encontrado: {m}"),
            Self::Exists(m) => write!(f, "já existe: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Origem do que foi lido: o arquivo principal ou a cópia válida anterior (principal ausente ou corrompido).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadStatus {
    Primary,
    RecoveredFromBackup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RevisionReason {
    /// Ajuste contínuo (slider, valor numérico) já agrupado pelo serviço.
    Adjustment,
    Structural,
    Import,
    Reset,
    Restore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionInfo {
    pub seq: u64,
    pub reason: RevisionReason,
    pub created_unix: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionFile {
    schema_version: u32,
    reason: RevisionReason,
    created_unix: u64,
    profile: ProfileDto,
}

pub struct Store {
    config_dir: PathBuf,
    state_dir: PathBuf,
}

fn backup_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

/// Grava por arquivo temporário na mesma pasta + fsync + rename. Antes de substituir, preserva o arquivo atual
/// como `.bak` apenas se ele ainda for válido (`is_valid`), para nunca trocar uma boa cópia por uma corrompida.
fn write_atomic(path: &Path, contents: &str, is_valid: impl Fn(&str) -> bool) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    if let Ok(previous) = fs::read_to_string(path) {
        if is_valid(&previous) {
            let bak = backup_path(path);
            let tmp_bak = dir.join(format!(".bak-{}.tmp", std::process::id()));
            fs::write(&tmp_bak, previous)?;
            fs::rename(&tmp_bak, bak)?;
        }
    }
    let tmp = dir.join(format!(
        ".{}-{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn check_schema(text: &str) -> Result<(), StoreError> {
    let table: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| StoreError::Parse(e.to_string()))?;
    match table
        .get("schema_version")
        .and_then(toml::Value::as_integer)
    {
        Some(v) if v > i64::from(SCHEMA_VERSION) => Err(StoreError::Schema {
            found: u32::try_from(v).unwrap_or(u32::MAX),
            supported: SCHEMA_VERSION,
        }),
        Some(v) if v >= 1 => Ok(()), // migrações de versões anteriores entram aqui quando houver schema 2
        _ => Err(StoreError::Parse(
            "schema_version ausente ou inválido".into(),
        )),
    }
}

fn parse_profile(text: &str) -> Result<Profile, StoreError> {
    check_schema(text)?;
    let dto: ProfileDto = toml::from_str(text).map_err(|e| StoreError::Parse(e.to_string()))?;
    profile_from_dto(dto).map_err(StoreError::Invalid)
}

fn parse_config(text: &str) -> Result<GlobalConfig, StoreError> {
    check_schema(text)?;
    let cfg: GlobalConfig = toml::from_str(text).map_err(|e| StoreError::Parse(e.to_string()))?;
    cfg.validate().map_err(StoreError::Invalid)?;
    Ok(cfg)
}

/// Lê o principal; se ausente ou inválido, tenta a cópia anterior. Schema mais novo nunca cai para a cópia.
fn read_with_backup<T>(
    path: &Path,
    parse: impl Fn(&str) -> Result<T, StoreError>,
) -> Result<(T, LoadStatus), StoreError> {
    let primary = match fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err(StoreError::NotFound(path.display().to_string()))
        }
        Err(e) => Err(StoreError::Io(e)),
    };
    match primary {
        Ok(v) => Ok((v, LoadStatus::Primary)),
        Err(e @ StoreError::Schema { .. }) => Err(e),
        Err(first) => match fs::read_to_string(backup_path(path)).map(|t| parse(&t)) {
            Ok(Ok(v)) => Ok((v, LoadStatus::RecoveredFromBackup)),
            _ => Err(first),
        },
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn require_id(id: &str) -> Result<(), StoreError> {
    if is_valid_id(id) {
        Ok(())
    } else {
        Err(StoreError::Invalid(format!("id inválido: {id:?}")))
    }
}

impl Store {
    pub fn open(config_dir: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
            state_dir: state_dir.into(),
        }
    }

    /// `$XDG_CONFIG_HOME/iara` (ou `~/.config/iara`) e `$XDG_STATE_HOME/iara` (ou `~/.local/state/iara`).
    pub fn from_xdg() -> Result<Self, StoreError> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let pick = |var: &str, fallback: &str| -> Result<PathBuf, StoreError> {
            match std::env::var_os(var).filter(|v| !v.is_empty()) {
                Some(v) => Ok(PathBuf::from(v).join("iara")),
                None => home
                    .clone()
                    .map(|h| h.join(fallback).join("iara"))
                    .ok_or_else(|| StoreError::Invalid("HOME não definido".into())),
            }
        };
        Ok(Self::open(
            pick("XDG_CONFIG_HOME", ".config")?,
            pick("XDG_STATE_HOME", ".local/state")?,
        ))
    }

    fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    fn profile_file(&self, id: &str) -> PathBuf {
        self.config_dir.join("profiles").join(format!("{id}.toml"))
    }

    fn history_dir(&self, id: &str) -> PathBuf {
        self.state_dir.join("history").join(id)
    }

    /// Configuração global; sem arquivo, valores padrão (não grava).
    pub fn load_config(&self) -> Result<(GlobalConfig, LoadStatus), StoreError> {
        match read_with_backup(&self.config_file(), parse_config) {
            Err(StoreError::NotFound(_)) => Ok((GlobalConfig::default(), LoadStatus::Primary)),
            other => other,
        }
    }

    pub fn save_config(&self, cfg: &GlobalConfig) -> Result<(), StoreError> {
        cfg.validate().map_err(StoreError::Invalid)?;
        let text = toml::to_string_pretty(cfg).map_err(|e| StoreError::Invalid(e.to_string()))?;
        write_atomic(&self.config_file(), &text, |t| parse_config(t).is_ok())?;
        Ok(())
    }

    pub fn list_profiles(&self) -> Result<Vec<String>, StoreError> {
        let dir = self.config_dir.join("profiles");
        let mut ids = Vec::new();
        match fs::read_dir(&dir) {
            Ok(entries) => {
                for e in entries {
                    let name = e?.file_name().to_string_lossy().into_owned();
                    if let Some(id) = name.strip_suffix(".toml") {
                        if is_valid_id(id) {
                            ids.push(id.to_owned());
                        }
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        ids.sort();
        Ok(ids)
    }

    pub fn load_profile(&self, id: &str) -> Result<(Profile, LoadStatus), StoreError> {
        require_id(id)?;
        let (profile, status) = read_with_backup(&self.profile_file(id), parse_profile)?;
        if profile.id != id {
            return Err(StoreError::Invalid(format!(
                "o arquivo de {id} declara o id {}",
                profile.id
            )));
        }
        Ok((profile, status))
    }

    /// Valida o perfil inteiro (mesmas regras da carga) e grava de forma atômica.
    pub fn save_profile(&self, profile: &Profile) -> Result<(), StoreError> {
        require_id(&profile.id)?;
        plan(profile).map_err(|e| StoreError::Invalid(format!("{e:?}")))?;
        let text = toml::to_string_pretty(&profile_to_dto(profile))
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        parse_profile(&text)?; // garante que o que vai para o disco é relido pelo próprio schema
        write_atomic(&self.profile_file(&profile.id), &text, |t| {
            parse_profile(t).is_ok()
        })?;
        Ok(())
    }

    pub fn delete_profile(&self, id: &str) -> Result<(), StoreError> {
        require_id(id)?;
        let file = self.profile_file(id);
        if !file.exists() {
            return Err(StoreError::NotFound(id.to_owned()));
        }
        fs::remove_file(&file)?;
        let _ = fs::remove_file(backup_path(&file));
        let _ = fs::remove_dir_all(self.history_dir(id));
        Ok(())
    }

    /// Cópia independente com outro id e nome; nunca sobrescreve.
    pub fn duplicate_profile(
        &self,
        id: &str,
        new_id: &str,
        new_name: &str,
    ) -> Result<Profile, StoreError> {
        require_id(new_id)?;
        if self.profile_file(new_id).exists() {
            return Err(StoreError::Exists(new_id.to_owned()));
        }
        let (mut p, _) = self.load_profile(id)?;
        p.id = new_id.to_owned();
        p.name = new_name.to_owned();
        self.save_profile(&p)?;
        Ok(p)
    }

    pub fn export_profile(&self, id: &str, dest: &Path) -> Result<(), StoreError> {
        let (p, _) = self.load_profile(id)?;
        let text = toml::to_string_pretty(&profile_to_dto(&p))
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        write_atomic(dest, &text, |_| false)?;
        Ok(())
    }

    /// Valida antes de criar; se o id já existe, cria um novo (`id-2`, `id-3`…). Nunca sobrescreve um perfil.
    pub fn import_profile(&self, src: &Path) -> Result<Profile, StoreError> {
        let text = fs::read_to_string(src)?;
        let mut p = parse_profile(&text)?;
        let base = p.id.clone();
        let mut n = 1u32;
        while self.profile_file(&p.id).exists() {
            n += 1;
            let suffix = format!("-{n}");
            let keep = 32usize.saturating_sub(suffix.len());
            p.id = format!("{}{}", &base[..base.len().min(keep)], suffix);
        }
        if p.id != base {
            p.name = format!("{} (importado)", p.name);
        }
        self.save_profile(&p)?;
        Ok(p)
    }

    fn revision_seqs(&self, id: &str) -> Result<Vec<u64>, StoreError> {
        let mut seqs = Vec::new();
        match fs::read_dir(self.history_dir(id)) {
            Ok(entries) => {
                for e in entries {
                    let name = e?.file_name().to_string_lossy().into_owned();
                    if let Some(seq) = name
                        .strip_suffix(".toml")
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        seqs.push(seq);
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        seqs.sort_unstable();
        Ok(seqs)
    }

    fn revision_file(&self, id: &str, seq: u64) -> PathBuf {
        self.history_dir(id).join(format!("{seq:010}.toml"))
    }

    /// Guarda `previous` (o estado anterior à mudança) como revisão e descarta as mais antigas além de 50 (FIFO).
    pub fn push_revision(
        &self,
        previous: &Profile,
        reason: RevisionReason,
    ) -> Result<u64, StoreError> {
        require_id(&previous.id)?;
        plan(previous).map_err(|e| StoreError::Invalid(format!("{e:?}")))?;
        let seqs = self.revision_seqs(&previous.id)?;
        let seq = seqs.last().map_or(1, |s| s + 1);
        let file = RevisionFile {
            schema_version: SCHEMA_VERSION,
            reason,
            created_unix: now_unix(),
            profile: profile_to_dto(previous),
        };
        let text = toml::to_string_pretty(&file).map_err(|e| StoreError::Invalid(e.to_string()))?;
        write_atomic(&self.revision_file(&previous.id, seq), &text, |_| false)?;
        let mut all = seqs;
        all.push(seq);
        while all.len() > HISTORY_LIMIT {
            let oldest = all.remove(0);
            let _ = fs::remove_file(self.revision_file(&previous.id, oldest));
        }
        Ok(seq)
    }

    fn read_revision(&self, id: &str, seq: u64) -> Result<(RevisionInfo, Profile), StoreError> {
        let text = fs::read_to_string(self.revision_file(id, seq)).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                StoreError::NotFound(format!("revisão {seq} de {id}"))
            } else {
                e.into()
            }
        })?;
        check_schema(&text)?;
        let file: RevisionFile =
            toml::from_str(&text).map_err(|e| StoreError::Parse(e.to_string()))?;
        let info = RevisionInfo {
            seq,
            reason: file.reason,
            created_unix: file.created_unix,
        };
        let profile = profile_from_dto(file.profile).map_err(StoreError::Invalid)?;
        Ok((info, profile))
    }

    /// Revisões válidas, da mais antiga para a mais nova; arquivos ilegíveis são ignorados (não apagados).
    pub fn list_revisions(&self, id: &str) -> Result<Vec<RevisionInfo>, StoreError> {
        require_id(id)?;
        Ok(self
            .revision_seqs(id)?
            .into_iter()
            .filter_map(|seq| self.read_revision(id, seq).ok().map(|(info, _)| info))
            .collect())
    }

    pub fn load_revision(&self, id: &str, seq: u64) -> Result<Profile, StoreError> {
        require_id(id)?;
        self.read_revision(id, seq).map(|(_, p)| p)
    }

    /// Restaura uma revisão: guarda primeiro o estado atual no histórico e aplica a revisão como novo estado.
    pub fn restore_revision(&self, id: &str, seq: u64) -> Result<Profile, StoreError> {
        let target = self.load_revision(id, seq)?;
        let (current, _) = self.load_profile(id)?;
        self.push_revision(&current, RevisionReason::Restore)?;
        self.save_profile(&target)?;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iara_core::{initial_profile, DevicePreference, Gain};

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path().join("config"), dir.path().join("state"));
        (dir, s)
    }

    #[test]
    fn profile_round_trips_including_silence_and_devices() {
        let (_d, s) = store();
        let mut p = initial_profile();
        p.channels[0].personal.gain = Gain::SILENCE;
        p.channels[1].transmission.gain = Gain::from_db(-12.5).unwrap();
        p.channels[2].personal.muted = true;
        p.microphone.input_gain = Gain::SILENCE;
        p.chatmix.position = -0.25;
        p.preferred_output = Some(DevicePreference {
            persistent_key: "alsa_output.fone".into(),
        });
        s.save_profile(&p).unwrap();
        let (back, status) = s.load_profile("default").unwrap();
        assert_eq!(back, p);
        assert_eq!(status, LoadStatus::Primary);
        let text = fs::read_to_string(s.profile_file("default")).unwrap();
        assert!(text.contains("schema_version = 1") && text.contains("silence = true"));
        assert!(
            !text.to_lowercase().contains("inf"),
            "nunca serializar infinito"
        );
    }

    #[test]
    fn corrupted_primary_recovers_from_the_previous_valid_copy() {
        let (_d, s) = store();
        let mut p = initial_profile();
        s.save_profile(&p).unwrap();
        p.name = "Segundo".into();
        s.save_profile(&p).unwrap(); // a 1ª versão válida vira .bak
        fs::write(s.profile_file("default"), "isto = não é toml [").unwrap();
        let (back, status) = s.load_profile("default").unwrap();
        assert_eq!(status, LoadStatus::RecoveredFromBackup);
        assert_eq!(back.name, initial_profile().name);
        // regravar por cima de um principal corrompido não pode trocar a boa cópia por lixo
        s.save_profile(&p).unwrap();
        let bak = fs::read_to_string(backup_path(&s.profile_file("default"))).unwrap();
        assert!(parse_profile(&bak).is_ok());
    }

    #[test]
    fn newer_schema_is_refused_and_not_recovered_from_backup() {
        let (_d, s) = store();
        let p = initial_profile();
        s.save_profile(&p).unwrap();
        s.save_profile(&p).unwrap();
        let text = fs::read_to_string(s.profile_file("default"))
            .unwrap()
            .replace("schema_version = 1", "schema_version = 99");
        fs::write(s.profile_file("default"), text).unwrap();
        assert!(matches!(
            s.load_profile("default"),
            Err(StoreError::Schema {
                found: 99,
                supported: 1
            })
        ));
    }

    #[test]
    fn hostile_ids_and_values_are_rejected_before_touching_disk() {
        let (d, s) = store();
        for bad in ["../x", "a/b", "", "A", ".."] {
            assert!(
                matches!(s.load_profile(bad), Err(StoreError::Invalid(_))),
                "{bad}"
            );
        }
        let mut p = initial_profile();
        p.id = "../../etc/passwd".into();
        assert!(matches!(s.save_profile(&p), Err(StoreError::Invalid(_))));
        let mut p = initial_profile();
        p.channels[0].personal.gain = Gain::UNITY;
        p.chatmix.position = f64::NAN;
        assert!(s.save_profile(&p).is_err());
        assert!(
            !d.path().join("config").exists(),
            "nada deve ser criado em disco"
        );
        // ganho fora da faixa e campo desconhecido vindos de arquivo: o erro tem de ser o esperado, não qualquer um
        s.save_profile(&initial_profile()).unwrap();
        let ok = fs::read_to_string(s.profile_file("default")).unwrap();
        assert!(ok.contains("gain_db = 0.0"));
        fs::write(
            s.profile_file("default"),
            ok.replacen("gain_db = 0.0", "gain_db = 6.0", 1),
        )
        .unwrap();
        match s.load_profile("default") {
            Err(StoreError::Invalid(m)) => assert!(m.contains("entre -60 e 0"), "{m}"),
            other => panic!("esperava Invalid por faixa, veio {other:?}"),
        }
        fs::write(s.profile_file("default"), format!("{ok}\nsurpresa = 1\n")).unwrap();
        match s.load_profile("default") {
            Err(StoreError::Parse(m)) => assert!(m.contains("surpresa"), "{m}"),
            other => panic!("esperava Parse por campo desconhecido, veio {other:?}"),
        }
    }

    #[test]
    fn history_keeps_only_the_last_fifty_and_restore_preserves_the_current_state() {
        let (_d, s) = store();
        let mut p = initial_profile();
        s.save_profile(&p).unwrap();
        for i in 0..55u32 {
            p.name = format!("v{i}");
            s.push_revision(&p, RevisionReason::Adjustment).unwrap();
        }
        let revs = s.list_revisions("default").unwrap();
        assert_eq!(revs.len(), HISTORY_LIMIT);
        assert_eq!(
            revs.first().unwrap().seq,
            6,
            "as 5 mais antigas saíram (FIFO)"
        );
        assert_eq!(s.load_revision("default", 6).unwrap().name, "v5");
        // restaurar a revisão 10: o estado atual vai para o histórico e o perfil passa a ser o restaurado
        let mut current = initial_profile();
        current.name = "atual".into();
        s.save_profile(&current).unwrap();
        let restored = s.restore_revision("default", 10).unwrap();
        assert_eq!(restored.name, "v9");
        assert_eq!(s.load_profile("default").unwrap().0.name, "v9");
        let last = s.list_revisions("default").unwrap().pop().unwrap();
        assert_eq!(last.reason, RevisionReason::Restore);
        assert_eq!(s.load_revision("default", last.seq).unwrap().name, "atual");
        assert_eq!(s.list_revisions("default").unwrap().len(), HISTORY_LIMIT);
    }

    #[test]
    fn duplicate_import_and_export_never_overwrite() {
        let (d, s) = store();
        let mut p = initial_profile();
        p.channels[0].name = "Jogos".into();
        s.save_profile(&p).unwrap();
        let copy = s.duplicate_profile("default", "jogo", "Jogo").unwrap();
        assert_eq!((copy.id.as_str(), copy.name.as_str()), ("jogo", "Jogo"));
        assert!(matches!(
            s.duplicate_profile("default", "jogo", "x"),
            Err(StoreError::Exists(_))
        ));
        assert_eq!(s.list_profiles().unwrap(), ["default", "jogo"]);

        let file = d.path().join("export.toml");
        s.export_profile("default", &file).unwrap();
        let imported = s.import_profile(&file).unwrap();
        assert_eq!(imported.id, "default-2", "id em conflito recebe sufixo");
        assert!(imported.name.ends_with("(importado)"));
        assert_eq!(
            s.load_profile("default").unwrap().0.name,
            "Padrão",
            "o ativo não é sobrescrito"
        );

        let bad = d.path().join("bad.toml");
        fs::write(
            &bad,
            fs::read_to_string(&file)
                .unwrap()
                .replace("id = \"default\"", "id = \"x y\""),
        )
        .unwrap();
        assert!(s.import_profile(&bad).is_err());
        assert_eq!(
            s.list_profiles().unwrap().len(),
            3,
            "importação inválida não cria nada"
        );
    }

    #[test]
    fn config_defaults_roundtrip_and_validate() {
        let (_d, s) = store();
        let (cfg, _) = s.load_config().unwrap();
        assert_eq!(cfg, GlobalConfig::default());
        let mut cfg = cfg;
        cfg.active_profile = Some("default".into());
        cfg.share_output_device = true;
        cfg.shared_output_device = Some("alsa_output.fone".into());
        s.save_config(&cfg).unwrap();
        assert_eq!(s.load_config().unwrap().0, cfg);
        cfg.active_profile = Some("../x".into());
        assert!(s.save_config(&cfg).is_err());
    }

    #[test]
    fn delete_profile_removes_file_backup_and_history() {
        let (_d, s) = store();
        let p = initial_profile();
        s.save_profile(&p).unwrap();
        s.save_profile(&p).unwrap();
        s.push_revision(&p, RevisionReason::Structural).unwrap();
        s.delete_profile("default").unwrap();
        assert!(s.list_profiles().unwrap().is_empty());
        assert!(s.list_revisions("default").unwrap().is_empty());
        assert!(matches!(
            s.delete_profile("default"),
            Err(StoreError::NotFound(_))
        ));
    }
}

"""Auxiliar de teste da janela: lê a árvore de acessibilidade (AT-SPI) do `iara-ui` e aciona botões pelo nome acessível.

Uso: /usr/bin/python3 at.py dump [papéis,separados,por,vírgula] | act "nome do botão" [papel] | pos "nome"
Requer python-gobject e at-spi2-core (usar o python do sistema, não o do conda). No backend X11 as posições vêm zeradas.
"""
import sys, gi
gi.require_version('Atspi','2.0')
from gi.repository import Atspi
Atspi.init()

def app():
    d=Atspi.get_desktop(0)
    for i in range(d.get_child_count()):
        a=d.get_child_at_index(i)
        if a.get_name()=='iara-ui': return a

def walk(n, depth=0, out=None):
    out = [] if out is None else out
    try:
        role=n.get_role_name(); name=n.get_name() or ''
        ext=n.get_component_iface().get_extents(Atspi.CoordType.SCREEN) if n.get_component_iface() else None
        st=n.get_state_set()
        vis=st.contains(Atspi.StateType.SHOWING)
        out.append((depth, role, name, ext, vis, n))
        for i in range(n.get_child_count()):
            walk(n.get_child_at_index(i), depth+1, out)
    except Exception as e:
        pass
    return out

def dump(filter_roles=None, only_showing=True):
    for depth, role, name, ext, vis, _ in walk(app()):
        if only_showing and not vis: continue
        if filter_roles and role not in filter_roles: continue
        e = f"({ext.x},{ext.y} {ext.width}x{ext.height})" if ext else ""
        print("  "*depth + f"{role} {name!r} {e}")

def find(name, role=None):
    for depth, r, n, ext, vis, node in walk(app()):
        if vis and n==name and (role is None or r==role): return node, ext
    return None, None

def act(name, role=None):
    """Aciona o botão cujo nome acessível é `name` (ação 'click'/'toggle'/'activate'), nunca o rótulo dentro dele."""
    for depth, r, n, ext, vis, node in walk(app()):
        if not vis or n != name or r == 'label': continue
        if role and r != role: continue
        ai = node.get_action_iface()
        if not ai: continue
        names = [ai.get_action_name(k) for k in range(ai.get_n_actions())]
        for want in ('click', 'toggle', 'activate', 'press'):
            if want in names:
                ai.do_action(names.index(want)); return True
    print("não achei botão:", name); return False

if __name__=='__main__':
    cmd=sys.argv[1]
    if cmd=='dump': dump(set(sys.argv[2].split(',')) if len(sys.argv)>2 else None)
    elif cmd=='act': print('ok' if act(sys.argv[2], sys.argv[3] if len(sys.argv)>3 else None) else 'falhou')
    elif cmd=='chips':
        # etiquetas de aplicativos (botões 'NOME — estado (origem)') na ordem em que aparecem sob o rótulo da coluna
        out=[]; inside=False
        for depth, role, name, ext, vis, node in walk(app()):
            if not vis: continue
            if role=='label' and name==sys.argv[2]: inside=True; continue
            if inside and role=='label' and name in ('APLICATIVOS','NÃO ATRIBUÍDOS','MASTER','GAME','CHAT','MEDIA','AUX','MIC') and name!=sys.argv[2]: break
            if inside and role=='toggle button' and ' — ' in name and name.split(' — ')[0] and 'participa' not in name and 'silenciar' not in name: out.append(name.split(' — ')[0])
        print('\n'.join(out))
    elif cmd=='pos':
        n,e=find(sys.argv[2], sys.argv[3] if len(sys.argv)>3 else None)
        print(f"{e.x} {e.y} {e.width} {e.height}" if e else "none")

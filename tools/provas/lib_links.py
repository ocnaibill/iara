#!/usr/bin/env python3
"""Auxiliar das provas: imprime, para cada nome de nó pedido, a que nós ele está ligado (via pw-dump)."""
import json, subprocess, sys

def dump():
    return json.loads(subprocess.run(["pw-dump"], capture_output=True, text=True).stdout)

def main(names):
    d = dump()
    nodes = {o["id"]: o["info"]["props"] for o in d if o["type"] == "PipeWire:Interface:Node"}
    links = [o["info"] for o in d if o["type"] == "PipeWire:Interface:Link"]
    by_name = {p.get("node.name"): i for i, p in nodes.items()}
    for n in names:
        i = by_name.get(n)
        short = n if len(n) < 34 else n[:14] + "…" + n[-17:]
        if i is None:
            print(f"   {short:<34} AUSENTE do grafo")
            continue
        ins = sorted({nodes.get(l["output-node-id"], {}).get("node.name", "?") for l in links if l["input-node-id"] == i})
        outs = sorted({nodes.get(l["input-node-id"], {}).get("node.name", "?") for l in links if l["output-node-id"] == i})
        sh = lambda xs: [x if len(x) < 34 else x[:14] + "…" + x[-17:] for x in xs]
        print(f"   {short:<34} recebe de {sh(ins) or '—'}  envia para {sh(outs) or '—'}")

if __name__ == "__main__":
    main(sys.argv[1:])

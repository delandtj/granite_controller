#!/usr/bin/env python3
"""JLC assembly stock for BOM lines, and a parts search, from JLC's own API.

The stock that matters for a JLC PCBA order is JLC's, not LCSC's: LCSC's
product endpoint hides stock from anonymous clients (it answers 0 even for
basic parts) and jlcsearch no longer returns stock at all. JLC's parts
library search does, keyed by LCSC code or model name.

Usage:
    tools/jlc_stock.py check NAME=BOM.csv=QTY [NAME=BOM.csv=QTY ...]
        Every BOM line (JLC BOM csv: Comment, Designator, Footprint,
        LCSC Part #) against QTY boards. Prints the lines under 1.2x the
        need (LOW / SHORT) and the parts shared between boards whose total
        demand is short. Exit 1 if anything is SHORT.
    tools/jlc_stock.py search KEYWORD [KEYWORD ...]
        JLC library search; prints code, model, package, stock, basic /
        extended. Use model families ("TMP75", "INA228AIDGSR") or LCSC
        codes.

Example (FD batch 2, fab-archive/fd-batch2/README.md):
    tools/jlc_stock.py check \\
      controller-fd=fab-archive/fd-batch2/controller-fd/granite-controller-fd-bom.csv=5 \\
      core-fd=fab-archive/fd-batch2/pp-48-core-fd/pp-48-core-fd-bom.csv=10 \\
      unit-fd=fab-archive/fd-batch2/pp-48-unit-fd/pp-48-unit-fd-bom.csv=30 \\
      pm-b=fab-archive/fd-batch2/pm-19v-250w-b/pm-19v-250w-b-bom.csv=50

The API is undocumented and may change; if every line comes back with no
stock, check one known basic part (C25744) before trusting the result.
"""

import csv
import json
import sys
import time
import urllib.request

API = "https://jlcpcb.com/api/overseas-pcb-order/v1/shoppingCart/smtGood/selectSmtComponentList"


def search(keyword, page_size=25):
    body = json.dumps({"keyword": keyword, "currentPage": 1, "pageSize": page_size}).encode()
    req = urllib.request.Request(API, data=body, headers={
        "User-Agent": "Mozilla/5.0", "Content-Type": "application/json"})
    data = json.load(urllib.request.urlopen(req, timeout=20))
    time.sleep(0.5)
    return ((data.get("data") or {}).get("componentPageInfo") or {}).get("list") or []


def stock_of(code, cache={}):
    if code not in cache:
        try:
            hit = [x for x in search(code, 5) if x.get("componentCode") == code]
            cache[code] = (hit[0].get("stockCount"), hit[0].get("componentModelEn"),
                           hit[0].get("componentLibraryType")) if hit else (None, "not in JLC library", "")
        except Exception as err:  # network or API change
            cache[code] = (None, "error: %s" % err, "")
    return cache[code]


def check(specs):
    lines, short = [], False
    for spec in specs:
        name, path, qty = spec.split("=")
        for row in csv.DictReader(open(path, newline="")):
            code = row["LCSC Part #"].strip()
            count = len([d for d in row["Designator"].split(",") if d.strip()])
            need = count * int(qty)
            stock, model, kind = stock_of(code) if code else (None, "no LCSC number", "")
            lines.append((name, code, need, stock, model, kind, row["Comment"], row["Designator"]))
    for name, code, need, stock, model, kind, comment, des in lines:
        if stock is None or stock < need:
            flag, short = "SHORT", True
        elif stock < need * 1.2:
            flag = "LOW"
        else:
            continue
        print("%-5s %-14s %-10s need %5d stock %-7s %s %s [%s] %s" % (
            flag, name, code, need, stock, comment, kind, des[:40], model))
    totals = {}
    for name, code, need, stock, *_ in lines:
        if code:
            t = totals.setdefault(code, [0, stock, set()])
            t[0] += need
            t[2].add(name)
    for code, (need, stock, names) in sorted(totals.items()):
        if len(names) > 1 and (stock is None or stock < need * 1.2):
            short = short or stock is None or stock < need
            print("SHARED %-10s need %5d stock %-7s %s" % (code, need, stock, ",".join(sorted(names))))
    print("checked %d lines" % len(lines))
    return 1 if short else 0


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("check", "search"):
        raise SystemExit(__doc__)
    if sys.argv[1] == "check":
        return check(sys.argv[2:])
    for kw in sys.argv[2:]:
        print("==", kw)
        for x in search(kw):
            print("  %-10s %-30s %-16s stock %-8s %s" % (
                x.get("componentCode"), (x.get("componentModelEn") or "")[:30],
                (x.get("componentSpecificationEn") or "")[:16], x.get("stockCount"),
                x.get("componentLibraryType")))
    return 0


if __name__ == "__main__":
    sys.exit(main())

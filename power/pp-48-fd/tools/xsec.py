"""2D electrostatic cross-section solver (finite differences, nonuniform grid).

Microstrip on B.Cu (drawn as the top layer here) over the In2 plane.
Returns per-conductor C (with dielectric) and C0 (all air), and from them
single-ended, odd, even and differential impedance.
"""
import sys
import numpy as np
import scipy.sparse as sp
import scipy.sparse.linalg as spla

EPS0 = 8.8541878128e-12
C_LIGHT = 299792458.0


def grid(edges, fine, coarse, lo, hi):
    """nonuniform 1D grid: spacing `fine` within 0.15 mm of any edge, else up to `coarse`"""
    pts = [lo]
    while pts[-1] < hi:
        x = pts[-1]
        d = min(abs(x - e) for e in edges)
        step = fine if d < 0.15 else min(coarse, fine + 0.3 * (d - 0.15))
        pts.append(min(hi, x + step))
    pts = np.array(sorted(set(np.round(np.concatenate([pts, edges]), 6))))
    return pts[(pts >= lo) & (pts <= hi)]


def solve(geom, air=False, fine=0.004, coarse=0.1):
    h, er, t, tm, erm = geom["h"], geom["er"], geom["t"], geom["mask_t"], geom["mask_er"]
    if air:
        er = erm = 1.0
    ylo, yhi = geom["span"]
    ztop = geom["ztop"]
    rects = geom["rects"]           # (name, y0, y1)  name 'G' = ground
    yedges = [e for _, a, b in rects for e in (a, b)]
    zedges = [0.0, h, h + t, h + t + tm, h + tm]
    Y = grid(yedges, fine, coarse, ylo, yhi)
    Z = grid(zedges, fine, coarse * 3, 0.0, ztop)
    ny, nz = len(Y), len(Z)

    # cell permittivity (cell between Y[i],Y[i+1] and Z[j],Z[j+1]), by cell centre
    yc = (Y[:-1] + Y[1:]) / 2
    zc = (Z[:-1] + Z[1:]) / 2
    eps = np.ones((ny - 1, nz - 1))
    eps[:, zc < h] = er
    cu = np.zeros(ny - 1, bool)
    for _, a, b in rects:
        cu |= (yc > a) & (yc < b)
    # conformal mask: above bare dielectric (h..h+tm), above copper (h+t..h+t+tm)
    for i in range(ny - 1):
        if cu[i]:
            eps[i, (zc > h + t) & (zc < h + t + tm)] = erm
        else:
            eps[i, (zc > h) & (zc < h + tm)] = erm

    # node labels: -1 free; k >= 0 fixed to conductor k; ground index 0
    names = ["GND"] + sorted({n for n, _, _ in rects if n != "G"})
    lab = -np.ones((ny, nz), int)
    lab[:, 0] = 0                    # plane
    lab[:, -1] = 0                   # far box
    for n, a, b in rects:
        k = 0 if n == "G" else names.index(n)
        lab[np.ix_((Y >= a - 1e-9) & (Y <= b + 1e-9), (Z >= h - 1e-9) & (Z <= h + t + 1e-9))] = k

    dy = np.diff(Y)
    dz = np.diff(Z)
    idx = lambda i, j: i * nz + j

    def coup(i, j, di, dj):
        """coupling coefficient between node (i,j) and (i+di, j+dj)"""
        if di:
            ci = i if di > 0 else i - 1
            L = dy[ci]
            w = 0.0
            if j > 0:
                w += eps[ci, j - 1] * dz[j - 1] / 2
            if j < nz - 1:
                w += eps[ci, j] * dz[j] / 2
            return w / L
        cj = j if dj > 0 else j - 1
        L = dz[cj]
        w = 0.0
        if i > 0:
            w += eps[i - 1, cj] * dy[i - 1] / 2
        if i < ny - 1:
            w += eps[i, cj] * dy[i] / 2
        return w / L

    free = np.argwhere(lab == -1)
    fid = -np.ones((ny, nz), int)
    fid[free[:, 0], free[:, 1]] = np.arange(len(free))
    rows, cols, vals = [], [], []
    nsig = len(names)
    B = np.zeros((len(free), nsig))
    for r, (i, j) in enumerate(free):
        diag = 0.0
        for di, dj in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            ii, jj = i + di, j + dj
            if not (0 <= ii < ny and 0 <= jj < nz):
                continue                 # Neumann at the sides
            c = coup(i, j, di, dj)
            diag += c
            if lab[ii, jj] == -1:
                rows.append(r); cols.append(fid[ii, jj]); vals.append(-c)
            else:
                B[r, lab[ii, jj]] += c
        rows.append(r); cols.append(r); vals.append(diag)
    A = sp.csc_matrix((vals, (rows, cols)), shape=(len(free), len(free)))
    lu = spla.splu(A)

    sig = names[1:]
    Cm = np.zeros((len(sig), len(sig)))
    fixed = np.argwhere(lab >= 0)
    for a, na in enumerate(sig):
        Vfix = np.zeros(nsig); Vfix[names.index(na)] = 1.0
        V = np.zeros((ny, nz))
        V[fixed[:, 0], fixed[:, 1]] = Vfix[lab[fixed[:, 0], fixed[:, 1]]]
        V[free[:, 0], free[:, 1]] = lu.solve(B @ Vfix)
        # charge on each conductor = sum of flux out of its nodes into free nodes
        for b_, nb in enumerate(sig):
            k = names.index(nb)
            q = 0.0
            for i, j in np.argwhere(lab == k):
                for di, dj in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    ii, jj = i + di, j + dj
                    if 0 <= ii < ny and 0 <= jj < nz and lab[ii, jj] != k:
                        q += coup(i, j, di, dj) * (V[i, j] - V[ii, jj])
            Cm[b_, a] = q * EPS0 * 1e-3 / 1e-3   # per metre (2D, mm units cancel)
    return sig, Cm, (ny, nz)


def impedances(geom, **kw):
    sig, C, n = solve(geom, **kw)
    _, C0, _ = solve(geom, air=True, **kw)
    L = np.linalg.inv(C0) / C_LIGHT**2
    out = {"grid": n, "sig": sig}
    for k, s in enumerate(sig):
        out[f"Z0_{s}"] = np.sqrt(L[k, k] / C[k, k]) if len(sig) == 1 else None
    if len(sig) == 1:
        out["eeff"] = C[0, 0] / C0[0, 0]
        return out
    # odd / even mode, pair = first two signal conductors
    Lo = L[0, 0] - L[0, 1]; Co = C[0, 0] - C[0, 1]
    Lo2 = L[1, 1] - L[1, 0]; Co2 = C[1, 1] - C[1, 0]
    Le = L[0, 0] + L[0, 1]; Ce = C[0, 0] + C[0, 1]
    out["Zodd"] = (np.sqrt(Lo / Co) + np.sqrt(Lo2 / Co2)) / 2
    out["Zeven"] = np.sqrt(Le / Ce)
    out["Zdiff"] = np.sqrt(Lo / Co) + np.sqrt(Lo2 / Co2)
    out["Zcomm"] = out["Zeven"] / 2
    out["eeff_odd"] = (C[0, 0] - C[0, 1]) / (C0[0, 0] - C0[0, 1])
    out["Zse"] = [np.sqrt(L[k, k] / C[k, k]) for k in range(2)]
    return out

"""Placement helpers shared by place_unit.py and place_core.py (docs/adr/0005).

put(): a part at (x, y) absolute, rotation, side. fit(): a part by its
courtyard box (one edge or centre given per axis), for connectors whose
origin is a pin. chain(): a JST PH side-entry chain header on B with its
pin row vertical, pin 1 at the top or bottom and the mating face at the
given board edge.
"""
import pcbnew

F, B = 'F', 'B'


def mm(v):
    return pcbnew.FromMM(v)


def get(board, ref):
    fp = board.FindFootprintByReference(ref)
    if fp is None:
        raise SystemExit('no footprint %s' % ref)
    return fp


def put(board, ref, x, y, rot, side):
    fp = get(board, ref)
    if fp.IsFlipped() != (side == B):
        fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
    fp.SetOrientationDegrees(rot)
    fp.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
    return fp


def courtyard(fp):
    box = fp.GetCourtyard(pcbnew.B_CrtYd if fp.IsFlipped() else pcbnew.F_CrtYd).BBox()
    return tuple(pcbnew.ToMM(v) for v in (box.GetLeft(), box.GetTop(), box.GetRight(), box.GetBottom()))


def fit(board, ref, rot, side, left=None, right=None, cx=None, top=None, bottom=None, cy=None):
    """place so the courtyard's left / right / centre x and top / bottom / centre y land where given"""
    fp = put(board, ref, 0.0, 0.0, rot, side)
    fp.BuildCourtyardCaches()
    x0, y0, x1, y1 = courtyard(fp)
    dx = (left - x0) if left is not None else (right - x1) if right is not None else (cx - (x0 + x1) / 2)
    dy = (top - y0) if top is not None else (bottom - y1) if bottom is not None else (cy - (y0 + y1) / 2)
    fp.SetPosition(pcbnew.VECTOR2I(mm(dx), mm(dy)))
    return fp


def pad_xy(fp, num):
    for p in fp.Pads():
        if p.GetNumber() == num:
            return pcbnew.ToMM(p.GetPosition().x), pcbnew.ToMM(p.GetPosition().y)
    raise SystemExit('%s has no pad %s' % (fp.GetReference(), num))


def chain(board, ref, edge_x, face, pin1_y, pin1_top):
    """PH 12 side-entry header on B: pin row vertical, pin 1 at pin1_y (top end if pin1_top), the
    mating face (courtyard edge) at edge_x, facing 'left' or 'right'"""
    for rot in (0, 90, 180, 270):
        fp = put(board, ref, 0.0, 0.0, rot, B)
        fp.BuildCourtyardCaches()
        (ax, ay), (bx, by) = pad_xy(fp, '1'), pad_xy(fp, '12')
        x0, y0, x1, y1 = courtyard(fp)
        body_left = (x0 + x1) / 2 < ax
        if abs(ax - bx) < 1e-3 and (ay < by) == pin1_top and body_left == (face == 'left'):
            dx = (edge_x - x0) if face == 'left' else (edge_x - x1)
            fp.SetPosition(pcbnew.VECTOR2I(mm(dx), mm(pin1_y - ay)))
            return fp
    raise SystemExit('%s: no orientation fits' % ref)


def solid(board, ref, num):
    for pad in get(board, ref).Pads():
        if pad.GetNumber() == num:
            pad.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL)

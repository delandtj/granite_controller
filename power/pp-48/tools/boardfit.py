"""Placement helpers shared by place_unit.py and place_core.py (docs/adr/0005).

put(): a part at (x, y) absolute, rotation, side. fit(): a part by its
courtyard box (one edge or centre given per axis), for connectors whose
origin is a pin. chain(): a JST PH side-entry chain header on B with its
pin row vertical, pin 1 at the top or bottom and the mating face at the
given board edge. link(): a right-angle 2x7 chain plug / socket on B, pin 1
at the top, its front (the plug's body, the socket's face) a given distance
past the board edge.
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
    """PH side-entry header on B: pin row vertical, pin 1 at pin1_y (top end if pin1_top), the
    mating face (courtyard edge) at edge_x, facing 'left' or 'right'"""
    last = str(max(int(p.GetNumber()) for p in get(board, ref).Pads() if p.GetNumber().isdigit()))
    for rot in (0, 90, 180, 270):
        fp = put(board, ref, 0.0, 0.0, rot, B)
        fp.BuildCourtyardCaches()
        (ax, ay), (bx, by) = pad_xy(fp, '1'), pad_xy(fp, last)
        x0, y0, x1, y1 = courtyard(fp)
        body_left = (x0 + x1) / 2 < ax
        if abs(ax - bx) < 1e-3 and (ay < by) == pin1_top and body_left == (face == 'left'):
            dx = (edge_x - x0) if face == 'left' else (edge_x - x1)
            fp.SetPosition(pcbnew.VECTOR2I(mm(dx), mm(pin1_y - ay)))
            return fp
    raise SystemExit('%s: no orientation fits' % ref)


# right-angle 2x7 2.54 mm: the front (plug: body face, the pins go on 6 mm; socket: entry face) from the
# outer pad row (the even pins, nearer the edge); KiCad's footprints, which match CJT A2541WR / A2541HWR
FRONT = {'PinHeader_2x07_P2.54mm_Horizontal': 4.04, 'PinSocket_2x07_P2.54mm_Horizontal': 10.03}


def link(board, ref, edge_x, face, pin1_y, past=0.0):
    """2x7 right-angle chain plug / socket on B: pin 1 at pin1_y at the top, rows vertical, the front
    `past` mm beyond the board edge edge_x, facing 'left' or 'right'"""
    fp = get(board, ref)
    front = FRONT[fp.GetFPID().GetLibItemName().wx_str()]
    for rot in (0, 90, 180, 270):
        fp = put(board, ref, 0.0, 0.0, rot, B)
        (ax, ay), (bx, by), (ox, _) = pad_xy(fp, '1'), pad_xy(fp, '13'), pad_xy(fp, '2')
        outer = ox > ax if face == 'right' else ox < ax
        if abs(ax - bx) < 1e-3 and ay < by and outer:
            s = 1 if face == 'right' else -1
            fp.SetPosition(pcbnew.VECTOR2I(mm(edge_x + s * past - s * front - ox), mm(pin1_y - ay)))
            return fp
    raise SystemExit('%s: no orientation fits' % ref)


def solid(board, ref, num):
    for pad in get(board, ref).Pads():
        if pad.GetNumber() == num:
            pad.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL)

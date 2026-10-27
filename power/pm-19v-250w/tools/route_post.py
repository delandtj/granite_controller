"""After krt-route: put back the hand routes of the nets krt-route rips
(tools/route_input.py POST_NETS), removing the router's copper on them
and its ground taps in their corner.
Run: pcbkit handroute <board> tools/route_post.py [--save]
"""
import os

_plan = {}
exec(open(os.path.join('tools', 'route_input.py')).read(), _plan)   # handroute runs from the project root
REGION = _plan['REGION']
# the router's ground taps in the compensation corner sit where these nets go
# back; those pads take ground from the F pour (pcbkit gndpour) or POST_GND
OPS = [('delreg', n, None, (100.0, 100.0, 158.4, 122.9)) for n in _plan['POST_NETS']] + \
      [('delreg', 'GND', 'F.Cu', (104.5, 119.8, 113.5, 122.9))] + \
      [op for op in _plan['OPS'] if op[0] in ('trk', 'via') and op[1] in _plan['POST_NETS']] + \
      _plan['POST_GND']

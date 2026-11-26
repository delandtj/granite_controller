const fs = require('fs');
const html = fs.readFileSync(process.argv[2], 'utf8');
const script = html.slice(html.lastIndexOf('<script>') + 8, html.lastIndexOf('</script>'));
const ids = [...html.matchAll(/id="([^"]+)"/g)].map(m => m[1]);
const els = {};
function mk(id) {
  const kids = [];
  const m = html.match(new RegExp('id="' + id + '"[^>]*>([\\s\\S]*?)</div>'));
  if (m) for (const b of m[1].matchAll(/<button[^>]*>/g)) {
    const attrs = {}; for (const a of b[0].matchAll(/([\w-]+)="([^"]*)"/g)) attrs[a[1]] = a[2];
    kids.push({ attrs, getAttribute(k) { return this.attrs[k]; }, setAttribute(k, v) { this.attrs[k] = v; }, addEventListener(e, f) { this.click = f; } });
  }
  const v = html.match(new RegExp('id="' + id + '"[^>]*value="([^"]+)"'));
  return { id, innerHTML: '', textContent: '', hidden: false, value: v ? v[1] : '', kids,
    querySelectorAll() { return this.kids; }, addEventListener(e, f) { this.on = f; }, setAttribute() {}, getAttribute() { return null; } };
}
ids.forEach(i => els[i] = mk(i));
global.document = { getElementById: i => els[i] || (els[i] = mk(i)) };
eval(script);
function report(tag) {
  const all = ['top', 'front', 'node', 'block', 'link', 'wiring', 'options', 'cands', 'chain', 'sched', 'pwrStats', 'sld', 'pwrSched', 'pwrFlag'].map(k => els[k].innerHTML).join('') + els.pwrLede.textContent + els.sldNote.textContent + els.pwrTitle.textContent + els.topNote.textContent + els.frontNote.textContent + els.fitFlag.innerHTML;
  const bad = (all.match(/NaN|undefined|Infinity/g) || []).length;
  const sel = els.options.innerHTML.match(/aria-pressed="true">[\s\S]*?<\/button>/);
  const txt = sel ? sel[0].replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').slice(20, 260) : '';
  console.log(tag, '| bad tokens', bad, '|', txt);
}
for (const b of els.presets.kids) { b.click(); report('preset ' + b.attrs['data-p']); console.log('   top:', els.topNote.textContent.slice(0, 230)); console.log('   front:', els.frontNote.textContent.slice(0, 160)); console.log('   power:', els.pwrTitle.textContent, '|', els.pwrLede.textContent.slice(140, 520), '|', els.pwrStats.innerHTML.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' '), '|', els.sldNote.textContent.slice(0, 150)); }
// cands for 48
els.presets.kids.find(b => b.attrs['data-p'] === 'n48').click();
console.log('CANDS:', els.cands.innerHTML.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').slice(0, 900));
console.log('FIT:', els.fitFlag.innerHTML.replace(/<[^>]+>/g, ''));
// orientation depth, widths
els.orient.kids.find(b => b.attrs['data-v'] === 'depth').click(); report('depth 198');
console.log('   top:', els.topNote.textContent.slice(0, 200));
els.fw.value = '180'; els.fw.on({ target: els.fw }); report('depth 180'); console.log('   top:', els.topNote.textContent.slice(0, 200));
els.pitch.value = '36'; els.pitch.on({ target: els.pitch }); report('depth 180 p36'); console.log('   top:', els.topNote.textContent.slice(0, 200));

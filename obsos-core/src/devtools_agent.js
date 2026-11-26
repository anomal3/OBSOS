// Агент инструментов разработчика OBSOS: ставится в страницу по требованию.
(function(){
if (window.__obsosDT) return;
var ids = new WeakMap(), nodes = new Map(), next = 1, box = null, picking = false;
function idOf(n){ var i = ids.get(n); if (!i) { i = next++; ids.set(n, i); nodes.set(i, n); } return i; }
function node(i){ var n = nodes.get(i); return n && n.isConnected !== false ? n : null; }
function ours(n){ return n && n.nodeType === 1 && n.hasAttribute && n.hasAttribute('data-obsos-dt'); }
function kids(n){
  var out = [], list = n.shadowRoot ? [n.shadowRoot] : [];
  for (var c = n.firstChild; c; c = c.nextSibling) list.push(c);
  list.forEach(function(c){
    if (ours(c)) return;
    if (c.nodeType === 3 && !c.nodeValue.trim()) return;
    if (c.nodeType === 10) return;
    out.push(c);
  });
  return out;
}
function describe(n, open, depth){
  if (n.nodeType === 3) return {i: idOf(n), k: 'text', x: n.nodeValue.trim().slice(0, 300)};
  if (n.nodeType === 8) return {i: idOf(n), k: 'comment', x: n.nodeValue.slice(0, 200)};
  if (n.nodeType === 11) return {i: idOf(n), k: 'shadow', c: kids(n).slice(0, 500).map(function(c){ return describe(c, open, depth + 1); })};
  var o = {i: idOf(n), k: 'element', t: n.localName, a: []};
  for (var j = 0; j < n.attributes.length && j < 40; j++) o.a.push([n.attributes[j].name, String(n.attributes[j].value).slice(0, 300)]);
  var c = kids(n);
  o.n = c.length;
  if (c.length === 1 && c[0].nodeType === 3 && c[0].nodeValue.trim().length < 80) { o.x = c[0].nodeValue.trim(); o.n = 0; }
  else if (c.length && (open[o.i] || depth < 2) && depth < 60) o.c = c.slice(0, 500).map(function(x){ return describe(x, open, depth + 1); });
  return o;
}
function rules(el){
  var out = [];
  function walk(list, sheetName, media){
    if (!list) return;
    for (var r = 0; r < list.length && out.length < 120; r++) {
      var rule = list[r];
      try {
        if (rule.selectorText) {
          if (el.matches(rule.selectorText)) out.push({s: rule.selectorText, css: rule.style.cssText, from: sheetName, media: media});
        } else if (rule.cssRules) {
          var cond = rule.conditionText || (rule.media && rule.media.mediaText) || '';
          if (!rule.media || window.matchMedia(rule.media.mediaText).matches) walk(rule.cssRules, sheetName, cond);
        } else if (rule.styleSheet) walk(rule.styleSheet.cssRules, sheetName, media);
      } catch (e) {}
    }
  }
  for (var s = 0; s < document.styleSheets.length; s++) {
    var sheet = document.styleSheets[s];
    var name = sheet.href ? sheet.href.split('/').pop().split('?')[0] : '<style>';
    try { walk(sheet.cssRules, name, ''); } catch (e) { out.push({s: '(таблица другого сайта — правила недоступны)', css: '', from: name, media: ''}); }
  }
  return out;
}
var COMMON = ['display','position','box-sizing','width','height','min-width','min-height','max-width','max-height','margin-top','margin-right','margin-bottom','margin-left','padding-top','padding-right','padding-bottom','padding-left','border-top-width','border-right-width','border-bottom-width','border-left-width','top','right','bottom','left','float','clear','overflow-x','overflow-y','z-index','opacity','visibility','color','background-color','background-image','font-family','font-size','font-weight','font-style','line-height','letter-spacing','text-align','text-transform','text-decoration-line','white-space','vertical-align','flex-direction','flex-wrap','flex-grow','flex-shrink','flex-basis','justify-content','align-items','align-self','row-gap','column-gap','grid-template-columns','grid-template-rows','transform','transition-duration','animation-name','cursor','pointer-events','border-top-left-radius','box-shadow'];
function details(i){
  var el = node(i);
  if (!el) return null;
  if (el.nodeType !== 1) el = el.parentElement || document.documentElement;
  var cs = getComputedStyle(el), r = el.getBoundingClientRect(), computed = [];
  COMMON.forEach(function(p){ var v = cs.getPropertyValue(p); if (v) computed.push([p, v]); });
  function four(a, b){ return ['top','right','bottom','left'].map(function(s){ return parseFloat(cs.getPropertyValue(a + '-' + s + (b || ''))) || 0; }); }
  var path = [];
  for (var e = el; e && e.nodeType === 1; e = e.parentElement) path.unshift(idOf(e));
  return {
    i: idOf(el), tag: el.localName, inline: el.getAttribute('style') || '',
    rect: [Math.round(r.x * 10) / 10, Math.round(r.y * 10) / 10, Math.round(r.width * 10) / 10, Math.round(r.height * 10) / 10],
    margin: four('margin'), border: four('border', '-width'), padding: four('padding'),
    computed: computed, rules: rules(el).reverse(), path: path,
    selector: el.localName + (el.id ? '#' + el.id : '') + (typeof el.className === 'string' && el.className.trim() ? '.' + el.className.trim().split(/\s+/).slice(0, 3).join('.') : '')
  };
}
function highlight(i){
  var el = node(i);
  if (!el || el.nodeType !== 1) el = el && el.parentElement;
  if (!el) { if (box) box.remove(); box = null; return; }
  var cs = getComputedStyle(el), r = el.getBoundingClientRect();
  function px(p){ return parseFloat(cs.getPropertyValue(p)) || 0; }
  var m = [px('margin-top'), px('margin-right'), px('margin-bottom'), px('margin-left')];
  var p = [px('padding-top'), px('padding-right'), px('padding-bottom'), px('padding-left')];
  var b = [px('border-top-width'), px('border-right-width'), px('border-bottom-width'), px('border-left-width')];
  if (box) box.remove();
  box = document.createElement('div');
  box.setAttribute('data-obsos-dt', '1');
  box.setAttribute('style', 'position:fixed;left:0;top:0;width:0;height:0;pointer-events:none;z-index:2147483647');
  function rect(x, y, w, h, color){
    var d = document.createElement('div');
    d.setAttribute('style', 'position:fixed;box-sizing:border-box;pointer-events:none;left:' + x + 'px;top:' + y + 'px;width:' + Math.max(0, w) + 'px;height:' + Math.max(0, h) + 'px;' + color);
    box.appendChild(d);
    return d;
  }
  rect(r.x - m[3], r.y - m[0], r.width + m[1] + m[3], r.height + m[0] + m[2], 'background:rgba(246,178,107,.35)');
  rect(r.x, r.y, r.width, r.height, 'background:rgba(147,196,125,.55)');
  rect(r.x + b[3] + p[3], r.y + b[0] + p[0], r.width - b[1] - b[3] - p[1] - p[3], r.height - b[0] - b[2] - p[0] - p[2], 'background:rgba(111,168,220,.75)');
  var label = rect(r.x, r.y > 26 ? r.y - m[0] - 24 : r.y + r.height + 4, 0, 0, '');
  label.setAttribute('style', 'position:fixed;pointer-events:none;white-space:nowrap;font:12px sans-serif;color:#fff;background:#222;border-radius:4px;padding:2px 8px;left:' + Math.max(0, r.x) + 'px;top:' + (r.y > 28 ? r.y - 24 : Math.max(0, r.y + r.height + 4)) + 'px');
  label.textContent = el.localName + (el.id ? '#' + el.id : '') + '  ' + Math.round(r.width) + ' × ' + Math.round(r.height);
  document.documentElement.appendChild(box);
}
function over(e){ if (picking && !ours(e.target)) highlight(idOf(e.target)); }
function choose(e){
  if (!picking) return;
  e.preventDefault(); e.stopPropagation(); e.stopImmediatePropagation();
  // Заканчиваем на click: иначе он дойдёт до страницы и откроет ссылку.
  if (e.type === 'click' || e.type === 'auxclick') {
    var el = e.target;
    pick(false);
    var path = [];
    for (var x = el; x && x.nodeType === 1; x = x.parentElement) path.unshift(idOf(x));
    console.log('__obsos_dt__:' + JSON.stringify({t: 'picked', i: idOf(el), path: path}));
  }
}
function pick(on){
  picking = on;
  var f = on ? 'addEventListener' : 'removeEventListener';
  window[f]('mousemove', over, true);
  ['mousedown','mouseup','click','auxclick','pointerdown','pointerup'].forEach(function(t){ window[f](t, choose, true); });
  if (!on) highlight(0);
}
window.__obsosDT = {
  pickAt: function(x, y){
    var el = document.elementFromPoint(x, y) || document.body;
    var path = [];
    for (var e = el; e && e.nodeType === 1; e = e.parentElement) path.unshift(idOf(e));
    setTimeout(function(){ console.log('__obsos_dt__:' + JSON.stringify({t: 'picked', i: idOf(el), path: path})); }, 0);
    return 1;
  },
  tree: function(open){ var set = {}; (open || []).forEach(function(i){ set[i] = true; }); return describe(document.documentElement, set, 0); },
  details: details,
  highlight: highlight,
  pick: pick,
  setAttr: function(i, name, value){ var el = node(i); if (el && el.setAttribute) { if (value === null) el.removeAttribute(name); else el.setAttribute(name, value); } return 1; },
  setText: function(i, text){ var n = node(i); if (n) n.textContent = text; return 1; },
  remove: function(i){ var n = node(i); if (n && n.parentNode) n.parentNode.removeChild(n); highlight(0); return 1; },
  outer: function(i){ var n = node(i); return n ? (n.outerHTML || n.nodeValue) : ''; },
  sources: function(){
    var out = [{k: 'doc', url: location.href, i: 0}];
    [].forEach.call(document.scripts, function(s, i){ out.push({k: 'js', url: s.src || '', i: i, size: s.src ? 0 : s.textContent.length}); });
    [].forEach.call(document.querySelectorAll('link[rel~=stylesheet], style'), function(s, i){ out.push({k: 'css', url: s.href || '', i: i, size: s.href ? 0 : s.textContent.length}); });
    return out;
  },
  source: function(kind, i){
    if (kind === 'doc') return document.documentElement.outerHTML;
    var e = kind === 'js' ? document.scripts[i] : document.querySelectorAll('link[rel~=stylesheet], style')[i];
    return e ? e.textContent : '';
  },
  storage: function(){
    function dump(s){ var o = []; try { for (var i = 0; i < s.length && i < 500; i++) { var k = s.key(i); o.push([k, String(s.getItem(k)).slice(0, 4000)]); } } catch (e) {} return o; }
    return {local: dump(window.localStorage), session: dump(window.sessionStorage), cookie: (function(){ try { return document.cookie; } catch (e) { return ''; } })(), origin: location.origin};
  },
  setStorage: function(area, key, value){ var s = area === 'local' ? localStorage : sessionStorage; if (value === null) s.removeItem(key); else s.setItem(key, value); return 1; },
  clearStorage: function(area){ (area === 'local' ? localStorage : sessionStorage).clear(); return 1; },
  perf: function(){
    var res = [], nav = null;
    try { res = performance.getEntriesByType('resource').map(function(e){ return [e.name, e.initiatorType || '', Math.round(e.startTime), Math.round(e.duration), e.transferSize || e.encodedBodySize || e.decodedBodySize || 0]; }); } catch (e) {}
    try {
      var t = performance.timing, s = t.navigationStart;
      if (s) nav = [['Ответ сервера', t.responseStart - s], ['HTML получен', t.responseEnd - s], ['DOM построен', t.domInteractive - s], ['DOMContentLoaded', t.domContentLoadedEventEnd - s], ['Событие load', t.loadEventEnd - s]];
    } catch (e) {}
    var paint = [];
    try { performance.getEntriesByType('paint').forEach(function(e){ paint.push([e.name, Math.round(e.startTime)]); }); } catch (e) {}
    return {nav: nav, paint: paint, res: res.slice(-600), elements: document.getElementsByTagName('*').length, ready: document.readyState, now: Math.round(performance.now())};
  }
};
})();

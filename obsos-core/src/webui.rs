//! Страницы управления браузером: `obsos://settings`, `obsos://history`,
//! `obsos://bookmarks`, `obsos://downloads`.
//!
//! Сам HTML данных не содержит: страница сообщает «готова», оболочка
//! (`webui_host.rs`) отдаёт ей состояние вызовом `obsos.state(…)` и
//! выполняет присланные команды — только если вкладка действительно открыта
//! на странице `obsos://` и прислала ключ этого запуска. Сайты ни прочитать
//! эти данные, ни управлять браузером так не могут.

/// Префикс сообщения страницы управления в консоли.
pub const PREFIX: &str = "__obsos_ui__:";

const CSS: &str = r#"
:root{--bg:#1c1d20;--panel:#292a2e;--card:#323337;--border:#414247;--text:#e6e7e9;--dim:#9b9ca1;--accent:#3b78d8;--accent2:#6aa6f8;--field:#1e1f22;--danger:#e5645f}
*{box-sizing:border-box}
html{background:var(--bg)}
body{margin:0;font-family:"Segoe UI",sans-serif;font-size:14px;line-height:1.45;color:var(--text);background:var(--bg);min-height:100vh}
.app{display:flex;min-height:100vh}
.nav{width:250px;flex:none;padding:26px 14px;background:var(--panel);border-right:1px solid var(--border)}
.brand{display:flex;align-items:center;gap:10px;font-size:20px;font-weight:700;margin:0 10px 22px}
.dot{width:14px;height:14px;border-radius:2px;background:var(--accent)}
.nav a{display:flex;align-items:center;gap:10px;padding:9px 14px;margin:2px 0;border-radius:4px;color:var(--text);cursor:pointer;text-decoration:none}
.nav a:hover{background:var(--card)}
.nav a.on{background:var(--accent);color:#fff}
.main{flex:1;min-width:0;padding:32px 36px 60px}
.col{max-width:820px;margin:0 auto}
h1{margin:0 0 20px;font-size:26px;font-weight:600}
h2{margin:26px 0 10px;font-size:13px;font-weight:600;letter-spacing:.6px;text-transform:uppercase;color:var(--dim)}
.card{background:var(--card);border:1px solid var(--border);border-radius:6px;margin-bottom:12px}
.row{display:flex;align-items:center;gap:16px;padding:14px 18px;border-top:1px solid var(--border)}
.row:first-child{border-top:0}
.row .t{flex:1;min-width:0}
.row .t b{display:block;font-weight:600}
.row .t span{display:block;color:var(--dim);font-size:12.5px;overflow:hidden;text-overflow:ellipsis}
.pad{padding:16px 18px}
.sw{flex:none;width:40px;height:22px;border-radius:4px;border:0;padding:0;background:var(--border);position:relative;cursor:pointer}
.sw i{position:absolute;left:3px;top:3px;width:16px;height:16px;border-radius:2px;background:#fff;transition:left .16s}
.sw.on{background:var(--accent)}
.sw.on i{left:21px}
.seg{display:flex;flex-wrap:wrap;gap:2px;padding:2px;background:var(--field);border-radius:4px;border:1px solid var(--border)}
.seg button{border:0;border-radius:3px;background:transparent;color:var(--text);padding:6px 12px;font:inherit;cursor:pointer}
.seg button:hover{background:var(--card)}
.seg button.on{background:var(--accent);color:#fff}
.btn{flex:none;border:1px solid var(--border);border-radius:4px;background:var(--panel);color:var(--text);padding:6px 14px;font:inherit;cursor:pointer;text-decoration:none}
.btn:hover{border-color:var(--accent)}
.btn.primary{background:var(--accent);border-color:var(--accent);color:#fff}
.btn.danger{color:var(--danger)}
.btn.small{padding:3px 10px;font-size:12.5px}
input.text{width:300px;max-width:100%;border:1px solid var(--border);border-radius:4px;background:var(--field);color:var(--text);padding:7px 10px;font:inherit}
input.text.short{width:100px}
input.text:focus{border-color:var(--accent);outline:none}
input.search{width:100%;margin-bottom:16px;padding:11px 16px;font-size:15px}
.cards{display:flex;flex-wrap:wrap;gap:12px;padding:16px 18px}
.pick{width:150px;padding:10px;border-radius:6px;border:2px solid var(--border);background:var(--panel);cursor:pointer;text-align:center}
.pick:hover{border-color:var(--accent2)}
.pick.on{border-color:var(--accent)}
.mini{height:74px;border-radius:3px;position:relative;overflow:hidden;margin-bottom:8px;background:#f2f3f8}
.mini i{position:absolute;display:block}
.swatches{display:flex;flex-wrap:wrap;gap:8px}
.swatch{width:26px;height:26px;border-radius:4px;border:3px solid transparent;cursor:pointer;padding:0}
.swatch.on{border-color:var(--text)}
.list .row{padding:9px 18px}
.list a.title{color:var(--text);text-decoration:none;display:block;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;cursor:pointer}
.list a.title:hover{color:var(--accent2)}
.list .when{flex:none;width:52px;color:var(--dim);font-size:12.5px}
.ico{flex:none;width:30px;height:30px;border-radius:4px;display:flex;align-items:center;justify-content:center;background:var(--panel);font-weight:700;color:var(--accent2)}
.bar{height:5px;border-radius:3px;background:var(--field);overflow:hidden;margin-top:8px}
.bar i{display:block;height:5px;background:var(--accent2);transition:width .3s}
.bar.wait i{width:30%;animation:slide 1.1s linear infinite}
@keyframes slide{from{margin-left:-30%}to{margin-left:100%}}
.empty{padding:34px;text-align:center;color:var(--dim)}
.note{color:var(--dim);font-size:12.5px;margin:6px 2px}
.keys{display:flex;flex-wrap:wrap}
.keys .row{width:50%;border-top:1px solid var(--border)}
kbd{flex:none;font-family:monospace;font-size:12px;background:var(--field);border:1px solid var(--border);border-radius:3px;padding:2px 8px}
@media (max-width:760px){
.app{display:block;min-height:0}
.nav{width:auto;display:flex;flex-wrap:wrap;gap:4px;padding:10px;border-right:0;border-bottom:1px solid var(--border)}
.brand{width:100%;margin:0 4px 6px;font-size:17px}
.nav a{margin:0;padding:6px 10px}
.main{padding:16px 12px 40px}
.row{flex-wrap:wrap}
.row .t{min-width:60%}
.keys .row{width:100%}
input.text{width:100%}
input.text.short{width:100px}
}
"#;

/// Общая часть сценария страниц: связь с оболочкой и построение разметки.
const COMMON_JS: &str = r#"
var S = null, last = '', waiting = false;
// Пока набирают в поле, страница не перерисовывается (набранное пропало бы).
function typing(){ var a = document.activeElement; return !!a && a.tagName === 'INPUT'; }
document.addEventListener('focusout', function(){ setTimeout(function(){ if (waiting && !typing()) { waiting = false; render(); } }, 250); });
function send(m){ m.token = TOKEN; console.log('__obsos_ui__:' + JSON.stringify(m)); }
function set(key, value){ send({cmd:'set', key:key, value:String(value)}); }
function act(name, arg, arg2){ send({cmd:'act', name:name, arg: arg == null ? '' : String(arg), arg2: arg2 == null ? '' : String(arg2)}); }
function h(tag, attrs){
  var e = document.createElement(tag);
  for (var k in attrs || {}) {
    var v = attrs[k];
    if (v == null || v === false) continue;
    if (k.indexOf('on') === 0) e.addEventListener(k.slice(2), v);
    else if (k === 'style') e.setAttribute('style', v);
    else if (k === 'value') e.value = v;
    else e.setAttribute(k, v === true ? '' : v);
  }
  for (var i = 2; i < arguments.length; i++) add(e, arguments[i]);
  return e;
}
function add(e, c){
  if (c == null || c === false) return;
  if (Array.isArray(c)) { c.forEach(function(x){ add(e, x); }); return; }
  e.appendChild(typeof c === 'object' ? c : document.createTextNode(String(c)));
}
function theme(t){
  var s = document.documentElement.style;
  for (var k in t) s.setProperty('--' + k, t[k]);
}
function row(title, hint, control){ return h('div', {class:'row'}, h('div', {class:'t'}, h('b', {}, title), hint ? h('span', {}, hint) : null), control); }
function card(){ var e = h('div', {class:'card'}); for (var i = 0; i < arguments.length; i++) add(e, arguments[i]); return e; }
function toggle(key, on){ return h('button', {class: 'sw' + (on ? ' on' : ''), role:'switch', 'aria-checked': on ? 'true' : 'false', onclick: function(){ set(key, !on); }}, h('i')); }
function seg(items, current, pick){ return h('div', {class:'seg'}, items.map(function(it){ return h('button', {class: it[0] === current ? 'on' : '', onclick: function(){ pick(it[0]); }}, it[1]); })); }
function btn(text, f, cls){ return h('button', {class: 'btn ' + (cls || ''), onclick: f}, text); }
function field(value, hint, commit, cls, type){
  var i = h('input', {class: 'text ' + (cls || ''), type: type || 'text', placeholder: hint || '', value: value || ''});
  var was = i.value;
  function done(){ if (i.value !== was) { was = i.value; commit(i.value); } }
  i.addEventListener('keydown', function(e){ if (e.key === 'Enter') { done(); i.blur(); } });
  i.addEventListener('blur', done);
  return i;
}
window.obsos = { state: function(s){
  var text = JSON.stringify(s);
  if (text === last) return;
  last = text; S = s; theme(s.theme);
  if (typeof HOLD !== 'undefined' && HOLD && typing()) { waiting = true; return; }
  render();
}};
send({cmd:'ready'});
"#;

const SETTINGS_JS: &str = r#"
var HOLD = true;
// Мастер-пароль — двумя полями и кнопкой: недонабранный не применится.
function masterPassword(){
  var a = h('input', {class:'text', type:'password', placeholder:'новый пароль', style:'width:200px'});
  var b = h('input', {class:'text', type:'password', placeholder:'ещё раз', style:'width:200px'});
  var note = h('span', {class:'note'});
  var apply = function(){
    if (!a.value) { note.textContent = 'Введите пароль'; return; }
    if (a.value !== b.value) { note.textContent = 'Пароли не совпадают'; return; }
    act('set_master', a.value); a.value = ''; b.value = ''; b.blur();
  };
  b.addEventListener('keydown', function(e){ if (e.key === 'Enter') apply(); });
  return h('div', {style:'display:flex;flex-direction:column;align-items:flex-end;gap:6px'}, a, b, note,
    h('div', {}, btn(S.has_master ? 'Сменить' : 'Задать', apply, 'primary small'), S.has_master ? btn('Снять', function(){ act('set_master', ''); }, 'small') : null));
}
function hex(key, title, value){ return row(title, value, field(value, '#rrggbb', function(v){ set(key, v); }, 'short')); }
var section = (location.search.match(/[?&]s=([a-z]+)/) || [0, 'look'])[1];
var SECTIONS = [['look','Внешний вид'],['search','Поиск и запуск'],['privacy','Конфиденциальность'],['downloads','Загрузки'],['sync','Синхронизация'],['profiles','Профили и скрипты'],['keys','Горячие клавиши'],['about','О программе']];
function go(id){ section = id; last = ''; render(); window.scrollTo(0, 0); }
function miniature(pos){
  var side = S.look.sidebar, acc = S.look.accent, bar = S.look.toolbar;
  var m = h('div', {class:'mini'});
  var box = function(css){ m.appendChild(h('i', {style: css})); };
  if (pos === 'left' || pos === 'right') {
    var at = pos === 'left' ? 'left:0;' : 'right:0;';
    box(at + 'top:0;width:34px;height:74px;background:' + side);
    for (var i = 0; i < 4; i++) box(at + 'margin:0 5px;top:' + (10 + i * 11) + 'px;width:24px;height:6px;border-radius:3px;background:' + (i ? '#8d93ad' : acc));
    box((pos === 'left' ? 'left:34px;' : 'left:0;') + 'top:0;right:' + (pos === 'left' ? '0' : '34px') + ';height:13px;background:' + bar);
  } else {
    var y = pos === 'top' ? 'top:0;' : 'bottom:0;';
    box(y + 'left:0;right:0;height:15px;background:' + side);
    for (var j = 0; j < 4; j++) box(y + 'margin:4px 0;left:' + (6 + j * 31) + 'px;width:27px;height:7px;border-radius:3px;background:' + (j ? '#8d93ad' : acc));
    box('left:0;right:0;top:' + (pos === 'top' ? '15px' : '0') + ';height:13px;background:' + bar);
  }
  return m;
}
function swatches(key, current){
  var colors = ['#7a4fd8','#5b3cc4','#3b82f6','#0ea5b7','#2bb3b1','#22a35a','#d97706','#e5645f','#d6458f','#64748b'];
  return h('div', {class:'swatches'}, colors.map(function(c){ return h('button', {class: 'swatch' + (c === current ? ' on' : ''), style: 'background:' + c, title: c, onclick: function(){ set(key, c); }}); }));
}
var PANES = {
  look: function(){
    var L = S.look;
    return [
      h('h2', {}, 'Положение вкладок'),
      card(h('div', {class:'cards'}, [['left','Слева'],['top','Сверху'],['right','Справа'],['bottom','Снизу']].map(function(p){
        return h('div', {class: 'pick' + (L.tabs === p[0] ? ' on' : ''), onclick: function(){ set('tabs', p[0]); }}, miniature(p[0]), p[1]); })),
        L.tabs === 'left' || L.tabs === 'right' ? row('Ширина боковой панели', null, seg([['180','Узкая'],['216','Обычная'],['280','Широкая'],['340','Очень широкая']], String(L.sidebar_width), function(v){ set('sidebar_width', v); })) : null),
      h('h2', {}, 'Тема'),
      card(h('div', {class:'cards'}, S.looks.map(function(l){
        var m = h('div', {class:'mini', style: 'background:' + l.toolbar});
        m.appendChild(h('i', {style: 'left:0;top:0;width:40px;height:74px;background:' + l.sidebar}));
        m.appendChild(h('i', {style: 'left:7px;top:10px;width:26px;height:8px;border-radius:4px;background:' + l.accent}));
        m.appendChild(h('i', {style: 'left:52px;top:12px;width:60px;height:7px;border-radius:3px;background:' + l.text}));
        m.appendChild(h('i', {style: 'left:52px;top:30px;width:38px;height:16px;border-radius:6px;background:' + l.accent2}));
        var remove = S.looks.length > 1 ? h('button', {class:'btn small', style:'margin-top:6px', onclick: function(e){ e.stopPropagation(); act('delete_look', l.name); }}, 'Удалить') : null;
        return h('div', {class: 'pick' + (l.name === L.name ? ' on' : ''), onclick: function(){ act('look', l.name); }}, m, l.name, h('div', {}, remove)); })),
        row('Основа', 'Заменяет цвета на тёмные или светлые', seg([['dark','Тёмная'],['light','Светлая']], L.dark ? 'dark' : 'light', function(v){ set('base', v); })),
        row('Основной цвет', null, swatches('accent', L.accent)),
        row('Второй цвет', null, swatches('accent2', L.accent2)),
        row('Куда заходит цвет', null, seg([['0','Выделение'],['1','И вкладка'],['2','И кнопки']], String(L.accent_reach), function(v){ set('accent_reach', v); })),
        row('Сохранить как свою тему', 'Темы уходят на другие устройства вместе с синхронизацией', field('', 'название и Enter', function(v){ act('save_look', v); }))),
      h('h2', {}, 'Свои цвета'),
      card(hex('sidebar', 'Боковая панель и полоса вкладок', L.sidebar), hex('toolbar', 'Панель адреса', L.toolbar), hex('text', 'Текст', L.text), hex('accent', 'Основной', L.accent), hex('accent2', 'Второй', L.accent2)),
      h('h2', {}, 'Размер'),
      card(
        row('Масштаб интерфейса', 'Кнопки, вкладки и меню. «Авто» — как в настройках экрана системы', seg([['0','Авто'],['100','100%'],['125','125%'],['150','150%'],['175','175%'],['200','200%']], String(S.ui_scale), function(v){ set('ui_scale', v); })),
        row('Масштаб страниц', 'Для всех сайтов; Ctrl+«+»/«−» меняет только текущую вкладку', seg([['80','80%'],['90','90%'],['100','100%'],['110','110%'],['125','125%'],['150','150%']], String(S.page_zoom), function(v){ set('page_zoom', v); })),
        row('Компактная панель адреса', null, toggle('compact', L.compact)))
    ];
  },
  search: function(){
    return [
      h('h2', {}, 'Поиск'),
      card(row('Поисковая система', 'Для запросов из адресной строки и новой вкладки', seg(S.engines, S.search, function(v){ set('search', v); }))),
      h('h2', {}, 'Запуск'),
      card(
        row('Домашняя страница', 'Alt+Home', field(S.home, 'obsos://newtab', function(v){ set('home', v); })),
        row('Открывать вкладки прошлого сеанса', null, toggle('restore_session', S.restore_session))),
      h('h2', {}, 'Страницы'),
      card(
        row('Плавная прокрутка колесом', null, toggle('smooth_scroll', S.smooth_scroll)),
        S.gpu_available ? row('Рисовать страницы видеокартой', (S.gpu_active ? 'Сейчас: ' + S.renderer : 'Сейчас: процессор') + ' · вступает в силу после перезапуска', toggle('gpu', S.gpu)) : null)
    ];
  },
  privacy: function(){
    return [
      h('h2', {}, 'Реклама и трекеры'),
      card(
        row('Блокировать рекламу и трекеры', 'Щит в адресной строке выключает блокировку на отдельном сайте', toggle('block_ads', S.block_ads)),
        row('Списки фильтров', S.filters, S.filters_busy ? h('span', {}, 'скачиваю…') : btn('Обновить', function(){ act('update_filters'); }))),
      h('h2', {}, 'Пароли'),
      card(
        row('Предлагать сохранить пароль', null, toggle('save_passwords', S.save_passwords)),
        row('Подставлять сохранённые пароли', null, toggle('autofill', S.autofill)),
        row('Сохранённые пароли', 'Просмотр, генератор, импорт и проверка утечек', btn('Открыть', function(){ act('panel', 'passwords'); })),
        row('Мастер-пароль', S.has_master ? 'Задан: OBSOS спрашивает его при запуске' : 'Не задан: хранилище открыто ключом этого устройства', masterPassword())),
      h('h2', {}, 'История'),
      card(
        row('Хранить историю, дней', null, field(String(S.history_days), '90', function(v){ set('history_days', v); }, 'short')),
        row('Журнал посещений', null, h('a', {class:'btn', href:'obsos://history'}, 'Открыть историю')),
        row('Удалить всю историю', 'На всех синхронизированных устройствах', btn('Очистить…', function(){ act('clear_history'); }, 'danger')))
    ];
  },
  downloads: function(){
    return [
      h('h2', {}, 'Загрузки'),
      card(
        row('Папка', S.download_dir, h('div', {}, btn('Изменить…', function(){ act('choose_dir'); }), S.download_dir_custom ? btn('Как было', function(){ set('download_dir', ''); }, 'small') : null)),
        row('Спрашивать, куда сохранять каждый файл', null, toggle('download_ask', S.download_ask)),
        row('Список загрузок', 'Ctrl+J', h('a', {class:'btn', href:'obsos://downloads'}, 'Открыть')))
    ];
  },
  sync: function(){
    return [
      h('h2', {}, 'Синхронизация'),
      card(
        row(S.sync_account ? 'Включена: ' + S.sync_account : 'Выключена', S.sync_status || 'Закладки, история, оформление и пароли — в ваш репозиторий или облако', btn(S.sync_account ? 'Площадка…' : 'Настроить…', function(){ act('panel', 'sync'); }, S.sync_account ? '' : 'primary')),
        S.sync_account ? row('Синхронизировать сейчас', 'Ctrl+Shift+S', btn('Запустить', function(){ act('sync_now'); })) : null,
        row('Синхронизировать самостоятельно', null, toggle('sync_auto', S.sync_auto)),
        row('Интервал, минут', null, field(String(S.sync_every_min), '15', function(v){ set('sync_every_min', v); }, 'short')),
        row('Ставить точку отката при каждой синхронизации', null, toggle('sync_points_auto', S.sync_points_auto))),
      h('h2', {}, 'Это устройство'),
      card(row('Имя устройства', 'Идентификатор ' + S.device, field(S.device_name, 'Этот компьютер', function(v){ set('device_name', v); })))
    ];
  },
  profiles: function(){
    return [
      h('h2', {}, 'Профили'),
      card(S.profiles.map(function(p){ return row(p, p === S.profile ? 'открыт сейчас' : null, p === S.profile ? h('span') : btn('Открыть в новом окне', function(){ act('profile', p); })); }),
        row('Новый профиль', 'Свои куки, закладки, пароли и синхронизация', field('', 'имя и Enter', function(v){ act('profile', v); })),
        row('Окно инкогнито', 'Ctrl+Shift+N', btn('Открыть', function(){ act('private'); }))),
      h('h2', {}, 'Скрипты и стили (вместо расширений)'),
      card(row('Папка', S.scripts_dir, h('div', {}, btn('Открыть папку', function(){ act('scripts_dir'); }), btn('Список', function(){ act('panel', 'scripts'); }, 'small'))))
    ];
  },
  keys: function(){
    var K = [['Ctrl+T','Новая вкладка'],['Ctrl+W','Закрыть вкладку'],['Ctrl+Shift+T','Вернуть закрытую'],['Ctrl+1…9','К вкладке по номеру'],['Ctrl+Tab','Следующая вкладка'],['Ctrl+L','Адресная строка'],['Ctrl+D','В закладки'],['Ctrl+H','История'],['Ctrl+J','Загрузки'],['Ctrl+Shift+O','Закладки'],['Ctrl+F','Найти на странице'],['Ctrl+R / F5','Обновить'],['Alt+← / →','Назад / вперёд'],['Alt+Home','Домашняя'],['Ctrl+ + / −','Масштаб'],['Ctrl+0','Масштаб 100%'],['F9','Режим чтения'],['F11','На весь экран'],['F12','Инструменты разработчика'],['Ctrl+Shift+C','Выбрать элемент'],['Ctrl+Shift+N','Инкогнито'],['Ctrl+,','Настройки'],['Ctrl+Shift+S','Синхронизировать'],['Ctrl+Shift+Q','Выход']];
    return [h('h2', {}, 'Клавиши'), card(h('div', {class:'keys'}, K.map(function(k){ return h('div', {class:'row'}, h('kbd', {}, k[0]), h('div', {class:'t'}, k[1])); })))];
  },
  about: function(){
    return [h('h2', {}, 'OBSOS'), card(
      row('Версия ' + S.version, 'Open Browser for System Open Source', h('a', {class:'btn', href:'obsos://about'}, 'Подробнее')),
      row('Движок', 'Servo 0.8 · SpiderMonkey · ' + S.renderer, h('a', {class:'btn', href:'obsos://licenses'}, 'Лицензии')),
      row('Профиль', S.profile_dir, btn('Открыть папку', function(){ act('profile_dir'); })))];
  }
};
function render(){
  if (!S) return;
  var root = document.getElementById('app');
  var title = SECTIONS.filter(function(s){ return s[0] === section; })[0] || SECTIONS[0];
  var nav = h('div', {class:'nav'}, h('div', {class:'brand'}, h('span', {class:'dot'}), 'Настройки'),
    SECTIONS.map(function(s){ return h('a', {class: s[0] === section ? 'on' : '', onclick: function(){ go(s[0]); }}, s[1]); }));
  var main = h('div', {class:'main'}, h('div', {class:'col'}, h('h1', {}, title[1]), (PANES[title[0]] || PANES.look)()));
  var y = window.scrollY;
  root.textContent = '';
  root.appendChild(nav); root.appendChild(main);
  window.scrollTo(0, y);
}
"#;

/// Список с поиском: история, закладки, загрузки.
const LIST_JS: &str = r#"
var query = '', shell = null;
function matches(){ var q = query.toLowerCase(); return function(x){ return !q || (x.title + ' ' + x.url).toLowerCase().indexOf(q) >= 0; }; }
function nav(on){
  return h('div', {class:'nav'}, h('div', {class:'brand'}, h('span', {class:'dot'}), 'OBSOS'),
    [['history','История'],['bookmarks','Закладки'],['downloads','Загрузки'],['settings','Настройки']].map(function(p){
      return h('a', {class: p[0] === on ? 'on' : '', href: 'obsos://' + p[0]}, p[1]); }));
}
function icon(x){ var t = (x.host || x.title || '?').replace(/^www\./, ''); return h('div', {class:'ico'}, t.charAt(0).toUpperCase()); }
function open(url){ return function(e){ e.preventDefault(); act((e.ctrlKey || e.button === 1) ? 'open_new' : 'open', url); }; }
function link(x){ return h('a', {class:'title', href: x.url, title: x.url, onclick: open(x.url)}, x.title || x.url); }
function render(){
  if (!S) return;
  if (!shell) {
    var root = document.getElementById('app');
    var search = h('input', {class:'text search', type:'text', placeholder: 'Поиск'});
    // История ищется на стороне браузера: здесь только последние записи.
    search.addEventListener('input', function(){ query = search.value; if (PAGE === 'history') act('search', query); render(); });
    shell = h('div');
    // Кнопки и поля сверху — один раз: перерисовка не сбивает набор.
    var tools = h('div', {style:'display:flex;gap:10px;margin-bottom:14px'});
    TOOLS(tools);
    root.appendChild(nav(PAGE));
    root.appendChild(h('div', {class:'main'}, h('div', {class:'col'}, h('h1', {}, TITLE), search, tools, shell)));
  }
  shell.textContent = '';
  VIEW(shell);
}
"#;

const HISTORY_JS: &str = r#"
var PAGE = 'history', TITLE = 'История';
function TOOLS(tools){ add(tools, btn('Очистить всё…', function(){ act('clear_history'); }, 'danger')); }
function VIEW(out){
  var items = S.items.filter(matches()), day = '', box = null;
  if (!items.length) { add(out, card(h('div', {class:'empty'}, query ? 'Ничего не нашлось' : 'История пуста'))); return; }
  items.slice(0, 600).forEach(function(x){
    if (x.day !== day) { day = x.day; add(out, h('h2', {}, day)); box = h('div', {class:'card list'}); add(out, box); }
    add(box, h('div', {class:'row'}, h('div', {class:'when'}, x.time), icon(x), h('div', {class:'t'}, link(x), h('span', {}, x.host)), btn('✕', function(){ act('forget', x.url); }, 'small')));
  });
  if (items.length > 600) add(out, h('div', {class:'note'}, 'Показаны первые 600 из ' + items.length + ' — уточните поиск.'));
}
"#;

const BOOKMARKS_JS: &str = r#"
var PAGE = 'bookmarks', TITLE = 'Закладки';
function TOOLS(tools){ add(tools, h('div', {class:'note'}, 'Ctrl+D добавляет текущую страницу. Импорт и экспорт — на панели паролей.')); }
function VIEW(out){
  var items = S.items.filter(matches());
  if (!items.length) { add(out, card(h('div', {class:'empty'}, query ? 'Ничего не нашлось' : 'Закладок пока нет — звёздочка в адресной строке или Ctrl+D'))); return; }
  var box = h('div', {class:'card list'}); add(out, box);
  items.forEach(function(x){ add(box, h('div', {class:'row'}, icon(x), h('div', {class:'t'}, link(x), h('span', {}, x.url)), btn('✕', function(){ act('unbookmark', x.url); }, 'small'))); });
}
"#;

const DOWNLOADS_JS: &str = r#"
var PAGE = 'downloads', TITLE = 'Загрузки';
function TOOLS(tools){
  add(tools, [btn('Открыть папку', function(){ act('downloads_dir'); }), btn('Очистить список', function(){ act('dl_clear'); }), field('', 'адрес файла и Enter — скачать', function(v){ act('download', v); })]);
}
function VIEW(out){
  add(out, h('div', {class:'note'}, 'Папка: ' + S.dir));
  var items = S.items.filter(matches());
  if (!items.length) { add(out, card(h('div', {class:'empty'}, 'Загрузок пока нет'))); return; }
  var box = h('div', {class:'card list'}); add(out, box);
  items.forEach(function(x){
    var bar = x.state === 'run' ? h('div', {class: 'bar' + (x.percent < 0 ? ' wait' : '')}, h('i', {style: x.percent < 0 ? '' : 'width:' + x.percent + '%'})) : null;
    var buttons = [];
    if (x.state === 'run') buttons.push(btn('Отменить', function(){ act('dl_cancel', x.id); }, 'small'));
    if (x.state === 'done') buttons.push(btn('Открыть', function(){ act('dl_open', x.id); }, 'small primary'), btn('В папке', function(){ act('dl_show', x.id); }, 'small'));
    if (x.state === 'fail') buttons.push(btn('Повторить', function(){ act('download', x.url); }, 'small'));
    if (x.state !== 'run') buttons.push(btn('✕', function(){ act('dl_remove', x.id); }, 'small'));
    add(box, h('div', {class:'row'}, h('div', {class:'ico'}, x.state === 'done' ? '✓' : x.state === 'fail' ? '!' : '↓'),
      h('div', {class:'t'}, h('b', {}, x.title), h('span', {}, x.status), h('span', {}, x.url), bar), buttons));
  });
}
"#;

fn document(title: &str, token: &str, script: &str, theme_css: &str) -> String {
    let mut quoted = String::new();
    browser_sync::json::write_str(token, &mut quoted);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>{CSS}{theme_css}</style></head>\
<body><div class=\"app\" id=\"app\"></div><script>(function(){{var TOKEN={quoted};\n{script}\n{COMMON_JS}}})();</script></body></html>"
    )
}

/// Страницы управления: заголовок и HTML по имени узла `obsos://<имя>`.
pub fn render(host: &str, token: &str, theme_css: &str) -> Option<String> {
    let (title, script) = match host {
        "settings" => ("Настройки", SETTINGS_JS.to_string()),
        "history" => ("История", format!("{LIST_JS}{HISTORY_JS}")),
        "bookmarks" => ("Закладки", format!("{LIST_JS}{BOOKMARKS_JS}")),
        "downloads" => ("Загрузки", format!("{LIST_JS}{DOWNLOADS_JS}")),
        _ => return None,
    };
    Some(document(title, token, &script, theme_css))
}

/// Имя узла — страница управления.
pub fn is_control_page(url: &url::Url) -> bool {
    url.scheme() == "obsos" && matches!(url.host_str(), Some("settings" | "history" | "bookmarks" | "downloads"))
}

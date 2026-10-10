// Reads a rendered page the way a person looks at it: what is drawn, what is
// on top, the words shown on or beside each control, and which boxes take
// text. Returns the page as an ordered list of controls and text, each with
// the containers a person would see it in. See `mod.rs` for the reply's shape.
//
// Called as `(root, limits) => reply`: `root` is a CSS selector to read
// under, or null for the whole page.
((root, limits) => {
  const base = root ? document.querySelector(root) : document.body;
  if (!base) return { ok: false, reason: 'root not found' };
  const width = window.innerWidth;
  const height = window.innerHeight;
  const squash = (text) => (text || '').replace(/\s+/g, ' ').trim();
  const clip = (text, most) => {
    const squashed = squash(text);
    return squashed.length > most ? squashed.slice(0, most - 1) + '…' : squashed;
  };
  const styles = new Map();
  const style = (element) => {
    if (!styles.has(element)) styles.set(element, getComputedStyle(element));
    return styles.get(element);
  };
  const boxes = new Map();
  const box = (element) => {
    if (!boxes.has(element)) boxes.set(element, element.getBoundingClientRect());
    return boxes.get(element);
  };
  const shown = (element) => {
    const rect = box(element);
    if (rect.width < 2 || rect.height < 2) return false;
    if (element.checkVisibility) {
      return element.checkVisibility({ opacityProperty: true, visibilityProperty: true });
    }
    const computed = style(element);
    return computed.display !== 'none' && computed.visibility !== 'hidden' && computed.opacity !== '0';
  };
  const TEXT_TYPES = ['text', 'search', 'email', 'tel', 'url', 'number', 'password'];
  // The most choices of one native dropdown offered as options: enough for a
  // title, a city or a month list, short of a country list's whole length.
  const OPTIONS_PER_DROPDOWN = 60;
  const TEXT_ROLES = ['textbox', 'searchbox', 'combobox', 'spinbutton'];
  const ROLES = [
    'button', 'link', 'checkbox', 'radio', 'switch', 'tab', 'menuitem', 'menuitemcheckbox',
    'menuitemradio', 'option', 'treeitem', 'slider', 'gridcell',
  ];
  const NESTED = 'a[href], button, input, select, textarea, [role="button"], [role="option"], '
    + '[role="link"], [role="combobox"], [role="menuitem"], [role="radio"], [role="checkbox"], '
    + '[role="tab"], [role="listbox"], [role="menu"], [role="dialog"]';
  const CARD_SELECTOR = 'li, [role="listitem"], [role="row"], article, [role="article"]';
  const MODAL_SELECTOR = 'dialog, [role="dialog"], [role="alertdialog"], [aria-modal="true"]';
  const tag = (element) => element.tagName.toLowerCase();
  const role = (element) => (element.getAttribute('role') || '').toLowerCase().split(' ')[0];
  // A page that greys a control out by style alone says so only in its class:
  // a calendar's past day is `rdrDay rdrDayDisabled`, pressable but inert.
  const DISABLED_CLASS = /disabled$/i;
  // And one it shows chosen, the same way: a store's picked size is
  // `size-buttons-size-button-selected`, with no ARIA state at all, so a
  // step choosing it never saw it chosen. Never `unselected`.
  const SELECTED_CLASS = /(?:^|[-_])(?:selected|checked)$/i;
  const UNSELECTED_CLASS = /(?:^|[-_])(?:un|not[-_]?)(?:selected|checked)$/i;
  // A class behind a variant (`placeholder:text-disabled`, which colours
  // only the placeholder; `disabled:opacity-50`, which applies only once
  // disabled) says nothing of the element's own state: live, a flight
  // site's place box carried `placeholder:text-disabled`, was read as
  // disabled, and its box was never seen.
  const stateClasses = (element) => [...element.classList].filter((name) => !name.includes(':'));
  const classChosen = (element) => stateClasses(element)
    .some((name) => SELECTED_CLASS.test(name) && !UNSELECTED_CLASS.test(name));
  const disabled = (element) =>
    element.disabled === true || element.getAttribute('aria-disabled') === 'true'
    || stateClasses(element).some((name) => DISABLED_CLASS.test(name));

  const insideText = (element) => {
    for (let parent = element; parent; parent = parent.parentElement) {
      if (tag(parent) === 'textarea' || parent.isContentEditable) return true;
    }
    return false;
  };

  // A box that takes typed text: a text-like input, a text area, or the
  // outermost editable region. Whatever role the page gives it.
  const takesText = (element) => {
    const name = tag(element);
    if (name === 'textarea') return !element.readOnly;
    if (name === 'input') {
      const type = (element.getAttribute('type') || 'text').toLowerCase();
      return TEXT_TYPES.includes(type) && !element.readOnly;
    }
    return element.isContentEditable
      && !(element.parentElement && element.parentElement.isContentEditable);
  };
  const FIELDS = 'input, textarea, [contenteditable=""], [contenteditable="true"]';
  // Whether a box to type in is drawn inside `element`.
  const holdsField = (element) => [...element.querySelectorAll(FIELDS)]
    .some((inner) => takesText(inner) && shown(inner));

  // The hidden checkbox or radio a label stands in for: pages draw their own
  // box and hide the real one — out of sight, or clipped away — and the
  // label is what a person clicks.
  const standIn = (element) => {
    if (tag(element) !== 'label') return null;
    const input = element.control;
    if (!input || tag(input) !== 'input' || !['checkbox', 'radio'].includes(input.type)) return null;
    const dropped = noiseRoot(input);
    return shown(input) && !(dropped && element.contains(dropped)) ? null : input;
  };

  const pointer = (element) => style(element).cursor === 'pointer';
  // A click handler a script framework keeps on the element itself (React
  // stores each element's props on it, Preact its listeners), on a box
  // smaller than a quarter of the window: a page can wire a plain `div` to a
  // click with neither a cursor nor a tab stop. Live, a store's "Add to
  // cart" and "Buy now" read as plain words, and the step to add to the cart
  // had nothing to press. Only a press handler: a carousel's track or a
  // select's menu listens for the mouse going down and is no button to press.
  const HANDLERS = ['onClick', 'onPress'];
  // Preact keeps them by event name (`_listeners`, `l` once minified), a
  // capture flag after it in newer releases ("clickfalse"). Live, a hotel
  // site's place suggestions were rows with only such a listener, read as
  // page text, and the place typed was never chosen.
  const PREACT_PRESS = /^click(true|false)?$/;
  const pressHandler = (element) => {
    const key = Object.keys(element).find((name) => name.startsWith('__reactProps$'));
    const props = key && element[key];
    if (props && HANDLERS.some((handler) => typeof props[handler] === 'function')) return true;
    const own = ['_listeners', 'l'].find((name) => Object.prototype.hasOwnProperty.call(element, name));
    const listeners = own && element[own];
    return !!listeners && typeof listeners === 'object' && Object.keys(listeners)
      .some((name) => PREACT_PRESS.test(name) && typeof listeners[name] === 'function');
  };
  // What is a control only by such a handler: it hides nothing pressable
  // inside it, since a page can wire a whole card to a click around its own
  // "Add" button.
  const scriptedOnly = new Set();
  const scripted = (element) => {
    if (!element || element === document.body || !pressHandler(element)) return false;
    const rect = element.getBoundingClientRect();
    return rect.width * rect.height < window.innerWidth * window.innerHeight * 0.25;
  };

  // A date picker's calendar: a table of day numbers under its month and
  // year. Many pickers draw a day as a plain cell that shows a pointer only
  // under the mouse, so nothing else marks it as pressable, yet a person
  // sees a day to pick. Each day cell maps to the date it stands for, and
  // each calendar's container is kept to read its paging arrows by.
  const MONTHS = ['january', 'february', 'march', 'april', 'may', 'june', 'july', 'august',
    'september', 'october', 'november', 'december'];
  const MONTH_AND_YEAR = new RegExp(`\\b(${MONTHS.join('|')})\\s+(\\d{4})\\b`, 'i');
  const calendarDays = new Map();
  const calendars = [];
  // The month and year a grid of days shows: the nearest short text before
  // it, or before one of its five nearest ancestors, that names one, as
  // `{ element, months: [[month, year], …] }`. A longer block (another
  // month's whole grid) ends the search at its level. A calendar already
  // `found` is passed over, however short, but a title beyond it names this
  // grid only while it has a month left to give (`uses` counts the grids
  // each title has named).
  const MONTHS_AND_YEARS = new RegExp(MONTH_AND_YEAR.source, 'gi');
  const gridTitle = (grid, found, uses) => {
    let node = grid;
    let passed = false;
    for (let depth = 0; node && node !== base && depth < 6; depth += 1, node = node.parentElement) {
      let sibling = node.previousElementSibling;
      for (let step = 0; sibling && step < 3; step += 1, sibling = sibling.previousElementSibling) {
        // A hidden element's text still reads out (a template, a month
        // menu): only what shows titles a grid.
        if (!shown(sibling)) continue;
        if (found.some((calendar) => sibling === calendar || sibling.contains(calendar))) {
          passed = true;
          continue;
        }
        const said = shownWords(sibling);
        if (said.length > 120) break;
        const months = [...said.matchAll(MONTHS_AND_YEARS)]
          .map((found) => [MONTHS.indexOf(found[1].toLowerCase()), Number(found[2])]);
        if (!months.length) continue;
        return passed && (uses.get(sibling) || 0) >= months.length
          ? null : { element: sibling, months };
      }
    }
    return null;
  };
  const findCalendars = () => {
    for (const table of base.querySelectorAll('table')) {
      // A week-number column is numbers too, but no day.
      const cells = [...table.querySelectorAll('td')].filter((cell) => /^\d{1,2}$/.test(squash(cell.textContent))
        && !/(^|\s)(cw|week)/i.test(cell.className));
      if (cells.length < 28 || !shown(table)) continue;
      // The month is named in the table's own heading, or in a short header
      // drawn just before it; never by words elsewhere on the page, nor by a
      // calendar that happens to come before it. A header is read as shown:
      // live, one held a hidden month list for the picker's own menu, whose
      // twelve names hid the month it showed.
      const before = table.previousElementSibling;
      const header = before && !before.querySelector('table') && shownWords(before).length <= 80
        ? before : null;
      const titled = MONTH_AND_YEAR.exec(squash([table.caption, table.tHead, header]
        .filter(Boolean).map((part) => shownWords(part)).join(' ')));
      if (!titled) continue;
      const holder = table.parentElement;
      calendars.push(holder && holder !== document.body && holder !== document.documentElement ? holder : table);
      const month = MONTHS.indexOf(titled[1].toLowerCase());
      const year = Number(titled[2]);
      // Days before the month's first belong to the month before, and days
      // after its last to the month after: the numbers start again.
      let offset = Number(squash(cells[0].textContent)) === 1 ? 0 : -1;
      let last = 0;
      for (const cell of cells) {
        const day = Number(squash(cell.textContent));
        if (day < last) offset += 1;
        last = day;
        const date = new Date(Date.UTC(year, month + offset, day));
        const spelled = MONTHS[date.getUTCMonth()];
        calendarDays.set(cell, `${day} ${spelled[0].toUpperCase()}${spelled.slice(1)} ${date.getUTCFullYear()}`);
      }
    }
    // A calendar drawn without a table: a grid whose cells each begin with
    // their day number (a fare may follow, "22 6529"), numbered from 1 to the
    // month's last day, below the month and year it shows. A title naming
    // two months, one header over two grids, names them in order. Live, a
    // flight site drew its days as buttons in such a grid, no day read as a
    // date, and the departure was never picked.
    const titleUses = new Map();
    const grids = [];
    const titled = [];
    for (const grid of base.querySelectorAll('div, ul, ol, tbody')) {
      // A month drawn as its weeks, each a row of up to seven days, is read
      // as the run of its days, as a flat grid is: four to six weeks, after
      // a row of day names when the month draws one there. Live, a hotel
      // site's open days held a fare and no month, inside week rows: none
      // read as a date, and a date step paged a year past the month it
      // wanted.
      const weeks = [...grid.children];
      const weekly = weeks.length >= 4 && weeks.length <= 7
        && weeks.every((week) => week.children.length >= 1 && week.children.length <= 7);
      const kids = weekly ? weeks.flatMap((week) => [...week.children]) : weeks;
      // The cheap tests first: a month's first day is among its first two
      // weeks' cells, read without laying the page out.
      if (kids.length < 28 || kids.length > 49
        || !kids.slice(0, 14).some((kid) => /^\s*1/.test(kid.textContent))
        || [...calendars, ...grids].some((calendar) => calendar.contains(grid))) continue;
      const days = kids.map((kid) => {
        const leading = /^(\d{1,2})(?:\s|$)/.exec(squash(kid.innerText));
        return leading && shown(kid) ? { cell: kid, day: Number(leading[1]) } : null;
      });
      const start = days.findIndex((entry) => entry && entry.day === 1);
      if (start < 0) continue;
      const run = [];
      for (const entry of days.slice(start)) {
        if (!entry || entry.day !== run.length + 1) break;
        run.push(entry);
      }
      const title = run.length >= 28 && gridTitle(grid, [...calendars, ...grids], titleUses);
      if (!title) continue;
      titleUses.set(title.element, (titleUses.get(title.element) || 0) + 1);
      grids.push(grid);
      titled.push({ run, title });
    }
    // A title's months go to its grids in order, and only when each month
    // found its grid: one grid missed (a "Today" over its first day, a week
    // hidden) would give the next grid's days the month before's name. Its
    // grids then stay calendars, with their arrows, and their days undated.
    const given = new Map();
    for (const { run, title } of titled) {
      const index = given.get(title.element) || 0;
      given.set(title.element, index + 1);
      if (titleUses.get(title.element) !== title.months.length) continue;
      const [month, year] = title.months[index];
      const spelled = MONTHS[month];
      for (const { cell, day } of run) {
        const pressed = cell.matches(NESTED) ? cell : cell.querySelector(NESTED) || cell;
        calendarDays.set(pressed, `${day} ${spelled[0].toUpperCase()}${spelled.slice(1)} ${year}`);
      }
    }
    // Each month is the block holding its grid, with its heading and
    // arrows, once every grid is found: a block holding two months' grids
    // side by side, with no box of each month's own, is no one month, and
    // is added as the picker below.
    for (const grid of grids) {
      const holder = grid.parentElement;
      const shared = holder && grids.some((other) => other !== grid && holder.contains(other));
      calendars.push(holder && holder !== document.body && !shared ? holder : grid);
    }
    // A picker showing two months beside each other pages both with one
    // pair of arrows, drawn beside the months rather than inside either:
    // the block that holds more than one month pages them too.
    for (const holder of [...calendars]) {
      const picker = holder.parentElement;
      if (picker && picker !== document.body && !calendars.includes(picker)
        && calendars.filter((other) => other !== picker && picker.contains(other)).length > 1) {
        calendars.push(picker);
      }
    }
    // A month's title is its calendar's too, and pages it with the arrows
    // drawn in it (live, a flight site's "‹ October 2026 – November 2026 ›"
    // bar sat above both months, outside either), unless it holds a form's
    // fields. Added last, so no block around a title and its grid is taken
    // for a picker of two months.
    for (const { title } of titled) {
      const block = title.element;
      if (!calendars.includes(block) && !block.querySelector('input, select, textarea')) {
        calendars.push(block);
      }
    }
  };
  // A month's name as a whole word, in full or cut short ("Oct", "Sept").
  const MONTH_WORD = /\b(?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sept?(?:ember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\b/i;
  // A calendar's paging arrow, read as what it does: an arrow glyph, or a
  // bare "Next", inside a calendar turns its month.
  const NEXT_GLYPHS = /^(?:next|[›»>→⟩▶❯])$/i;
  const PREVIOUS_GLYPHS = /^(?:prev|previous|[‹«<←⟨◀❮])$/i;
  const monthTurn = (element, name) => {
    if (!calendars.some((calendar) => calendar.contains(element))) return name;
    if (NEXT_GLYPHS.test(name)) return 'next month';
    if (PREVIOUS_GLYPHS.test(name)) return 'previous month';
    return name;
  };

  // What a person would take the element for, or null when it is not
  // something they would act on by itself.
  const kind = (element, insideControl) => {
    const name = tag(element);
    const claimed = role(element);
    if (takesText(element)) {
      return claimed === 'searchbox' || element.type === 'search' ? 'searchbox' : 'textbox';
    }
    if (name === 'input') {
      const type = (element.getAttribute('type') || 'text').toLowerCase();
      if (type === 'hidden') return null;
      if (type === 'checkbox') return claimed === 'switch' ? 'switch' : 'checkbox';
      if (type === 'radio') return 'radio';
      if (type === 'range') return 'slider';
      return 'button';
    }
    if (name === 'select') return 'combobox';
    if (standIn(element)) return standIn(element).type;
    if (TEXT_ROLES.includes(claimed)) {
      // A page's "text box" that holds no text box: a wrapper around the
      // real one, which is read instead, or a row or button to press.
      if (element.querySelector(FIELDS)) {
        return null;
      }
      return 'button';
    }
    if (ROLES.includes(claimed)) return claimed;
    if (name === 'a' && element.hasAttribute('href')) return 'link';
    if (name === 'button' || name === 'summary') return 'button';
    if (calendarDays.has(element)) return 'gridcell';
    // A region that holds controls (a menu, a list, a tab panel, a dialog)
    // takes a tab stop to move the focus inside it, not to be pressed: read
    // as one button, it would hide every row inside it. One a page makes
    // pressable itself, by its cursor or a click handler (a carousel's slide),
    // is still a button.
    if ((GROUP_ROLES.includes(claimed) || claimed === 'dialog' || claimed === 'alertdialog')
      && !element.hasAttribute('onclick') && !pointer(element)) return null;
    // Inside another control, what a press handler of its own wires is a
    // control too, as a native button inside a pressable card is: live, a
    // sort menu's options sat inside its pointer-cursor trigger, each wired
    // to a click by itself, and the menu read as one button naming them all.
    if (insideControl && !pressHandler(element)) return null;
    const tabindex = element.getAttribute('tabindex');
    const byPage = element.hasAttribute('onclick')
      || (tabindex !== null && tabindex !== '-1')
      || (pointer(element) && !(element.parentElement && pointer(element.parentElement)));
    const byScript = !byPage && scripted(element) && !scripted(element.parentElement);
    if (byScript) scriptedOnly.add(element);
    return byPage || byScript ? 'button' : null;
  };

  // Text of the elements `ids` (space-separated) names, looked up in
  // `scope`: the document, or the shadow root an element sits in.
  const byIds = (ids, scope = document) => squash((ids || '').split(/\s+/)
    .map((id) => id && scope.getElementById(id))
    .filter(Boolean)
    .map((element) => element.innerText || element.textContent)
    .join(' '));

  const ICON_WORDS = [
    'close', 'search', 'menu', 'back', 'next', 'previous', 'prev', 'forward', 'plus', 'minus',
    'add', 'remove', 'delete', 'edit', 'share', 'filter', 'sort', 'calendar', 'swap', 'cart',
    'account', 'user', 'profile', 'settings', 'home', 'help', 'info', 'play', 'pause', 'more',
    'expand', 'collapse', 'up', 'down', 'left', 'right', 'download', 'upload', 'refresh',
    'favorite', 'favourite', 'like', 'heart', 'star', 'bookmark', 'notification', 'bell',
    'logout', 'login', 'copy', 'print', 'mail', 'phone', 'location', 'map', 'clear', 'cancel',
    'increment', 'decrement', 'increase', 'decrease',
  ];
  // A picture-only control's meaning, from the words in its own or its
  // icon's class, id, or test id: all a person would see is the picture.
  // Words run together in camel case are words too ("icClose",
  // "closeIcon"): live, a sign-up pop-up's only way out was a sprite with
  // the class "icClose", dropped as a blank box, and nothing closed it. A
  // generated class ("fCarEc", "cUpXyz") has no two such words in a row.
  const iconWords = (element) => {
    const sources = [element, ...element.querySelectorAll('svg, use, i, img, span')].slice(0, 8);
    const words = new Set();
    for (const source of sources) {
      // A sprite's symbol ("#icon-cart") and a picture's file name
      // ("cart.svg") say what the icon shows too.
      const used = source.getAttribute('href') || source.getAttribute('xlink:href') || '';
      const file = tag(source) === 'img' ? (source.getAttribute('src') || '').split(/[?#]/)[0].split('/').pop() : '';
      const text = [
        typeof source.className === 'string' ? source.className
          : (source.className && source.className.baseVal) || '',
        source.id || '',
        source.getAttribute('data-testid') || '',
        source.getAttribute('data-icon') || '',
        tag(source) === 'use' ? used : '',
        file,
      ].join(' ').replace(/([a-z]{2,})(?=[A-Z][a-z]{2,})/g, '$1 ').toLowerCase();
      for (const word of text.split(/[^a-z]+/)) {
        if (ICON_WORDS.includes(word)) words.add(word);
      }
    }
    return [...words].slice(0, 3).join(' ');
  };

  // A stepper's unmarked picture buttons around the count they change ("−
  // 1 +" drawn as two icons): the one before the count lowers it, the one
  // after raises it. Live, a store's quantity buttons had no name at all,
  // and "increase the quantity to 2" pressed buttons at random.
  const stepperWord = (element) => {
    let parent = element.parentElement;
    for (let depth = 0; parent && depth < 3; depth += 1, parent = parent.parentElement) {
      const count = squash(parent.innerText);
      if (!/^\d{1,3}$/.test(count)) continue;
      if (parent.querySelectorAll('button, [role="button"]').length < 2) return '';
      const walker = document.createTreeWalker(parent, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        if (squash(node.data) !== count) continue;
        if (element.contains(node)) return '';
        const before = element.compareDocumentPosition(node) & Node.DOCUMENT_POSITION_FOLLOWING;
        return before ? 'decrease' : 'increase';
      }
      return '';
    }
    return '';
  };

  // The words `element` shows, without those of the dropdown it wraps (or
  // of `field`, the control a label names): a closed dropdown shows one
  // choice, but its text holds them all, so a label wrapping one would read
  // out every choice in it.
  const shownWords = (element, field) => {
    const left = field ? [field] : [...element.querySelectorAll('select')];
    if (!left.some((inner) => inner.firstChild && element.contains(inner))) return squash(element.innerText);
    const parts = [];
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (left.some((inner) => inner.contains(node)) || !node.parentElement.getClientRects().length) continue;
      parts.push(node.data);
    }
    return squash(parts.join(' '));
  };

  // Visible words that are not controls: each a candidate label for a
  // field beside or above it.
  const words = [];
  const collectWords = () => {
    const walker = document.createTreeWalker(base, NodeFilter.SHOW_TEXT);
    let seen = null;
    for (let node = walker.nextNode(); node && words.length < limits.labels; node = walker.nextNode()) {
      const parent = node.parentElement;
      if (!parent || parent === seen || !squash(node.data)) continue;
      seen = parent;
      if (!shown(parent) || insideText(parent)) continue;
      if (parent.closest(NESTED)) continue;
      const dropped = noiseRoot(parent);
      if (dropped && noiseKinds.get(dropped) === 'ads') continue;
      const text = clip(shownWords(parent) || node.data, 80);
      if (text) words.push({ element: parent, text, rect: box(parent) });
    }
  };

  // The words a person reads as a field's label: inside its box (a
  // floating label), to its left on the same line, or just above it; for a
  // checkbox or radio, just to its right.
  const DIVIDERS = /^(?:or|and|[^\p{L}\p{N}]*)$/iu;
  const nearby = (element, checkable) => {
    const field = box(element);
    let best = null;
    let bestGap = Infinity;
    for (const word of words) {
      const rect = word.rect;
      const across = Math.min(rect.bottom, field.bottom) - Math.max(rect.top, field.top);
      const along = Math.min(rect.right, field.right) - Math.max(rect.left, field.left);
      let gap = Infinity;
      const middleX = (rect.left + rect.right) / 2;
      const middleY = (rect.top + rect.bottom) / 2;
      if (middleX > field.left && middleX < field.right && middleY > field.top && middleY < field.bottom) {
        gap = 0;
      } else if (across > Math.min(rect.height, field.height) / 2 && rect.right <= field.left + 4) {
        gap = field.left - rect.right;
        if (gap > 200) gap = Infinity;
      } else if (along > 0 && rect.bottom <= field.top + 4) {
        gap = field.top - rect.bottom;
        gap = gap > 40 ? Infinity : gap + 1;
      } else if (checkable && across > 0 && rect.left >= field.right - 4) {
        gap = rect.left - field.right;
        if (gap > 40) gap = Infinity;
      }
      // A divider between two ways in ("OR") labels neither.
      if (gap < bestGap && word.text.length <= 60 && !DIVIDERS.test(word.text)) {
        best = word.text;
        bestGap = gap;
      }
    }
    return best;
  };

  // The words shown on the element itself, leaving out those of the
  // controls nested in it when it holds several: a field's button that holds
  // its open list of choices is named by the field, not by the choices. A
  // wrapper around one control is that control, and keeps its words.
  const ownText = (element) => {
    if (element.querySelectorAll(NESTED).length < 2) return squash(element.innerText);
    const parts = [];
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const parent = node.parentElement;
      const nested = parent && parent.closest(NESTED);
      if (nested && nested !== element && element.contains(nested)) continue;
      if (parent && shown(parent) && squash(node.data)) parts.push(squash(node.data));
      if (parts.join(' ').length > limits.name) break;
    }
    return squash(parts.join(' '));
  };

  // Letters an icon font draws as pictures: a person sees a magnifier or a
  // cross where the page stores "p" or "!". Live, a store's search and
  // close buttons read as "p" and "!", and the steps pressed them blindly.
  // Glyphs in Unicode's private use area are pictures in any font.
  const ICON_FONT = /icon|glyph|awesome|symbols|feather|icomoon/i;
  const PRIVATE_USE = /[\uE000-\uF8FF]/g;
  const SHORT_WORD = /(^| )\S{1,2}( |$)|[\uE000-\uF8FF]/;
  // A lone letter or two drawn in a font of its own, other than its
  // parent's, is a picture too, whatever the font is called (live, a
  // store's icon font had no telling name). Digits, currency signs, and
  // the signs a stepper or a close button shows as text are never pictures.
  const PICTURED = /^[^\p{N}\p{Sc}\s+\-−×✕<>‹›]$/u;
  // A font stack's own family, and the family a weight of it belongs to:
  // "Gilroy-SemiBold" inside "Gilroy-Regular" is the same text font, and a
  // fallback named in a stack ("Noto Sans Symbols") says nothing.
  const firstFamily = (font) => (font.split(',')[0] || '').replace(/["']/g, '').trim();
  const familyRoot = (font) => firstFamily(font).split(/[-\s_]/)[0].toLowerCase();
  const withoutGlyphs = (element, text) => {
    if (!SHORT_WORD.test(text)) return text;
    const glyphs = new Set();
    for (const part of [element, ...element.querySelectorAll('*')].slice(0, 30)) {
      const drawn = squash(part.textContent);
      if (!drawn || drawn.length > 2) continue;
      const font = style(part).fontFamily || '';
      const parent = part.parentElement;
      const own = parent && part !== element
        && familyRoot(font) !== familyRoot(style(parent).fontFamily || '');
      if (ICON_FONT.test(firstFamily(font)) || (own && PICTURED.test(drawn))) glyphs.add(drawn);
    }
    return squash(text.replace(PRIVATE_USE, ' ').split(' ')
      .filter((word) => !glyphs.has(word)).join(' '));
  };

  // The page's label on the one element inside a control that carries the
  // words it shows: a calendar day drawn as "18" whose inner span says
  // "Sunday, 18 October 2026", also when a fare follows its number ("23
  // 6757" labelled "October 23, 2026"). Several labels inside make it a
  // container, whose labels belong to what it holds. Live, two sites'
  // priced days carried their date only so, and none read as a date.
  const innerLabel = (element, text) => {
    const labelled = [...element.querySelectorAll('[aria-label]')];
    if (labelled.length !== 1) return '';
    const said = squash(labelled[0].getAttribute('aria-label'));
    if (said.includes(text)) return said;
    const day = /^(\d{1,2})(?:\s|$)/.exec(text);
    return day && MONTH_WORD.test(said) && new RegExp(`\\b${day[1]}\\b`).test(said) ? said : '';
  };

  // What a person reads as the element's name, and a description when the
  // page says more about it than it shows.
  const naming = (element, what) => {
    const aria = squash(element.getAttribute('aria-label'))
      || byIds(element.getAttribute('aria-labelledby'));
    const title = squash(element.getAttribute('title'));
    const input = standIn(element);
    if (['textbox', 'searchbox', 'combobox', 'slider'].includes(what) || (tag(element) === 'input' && !input)) {
      const labels = element.labels ? [...element.labels].map((label) => shownWords(label, element)).join(' ') : '';
      const checkable = ['checkbox', 'radio', 'switch'].includes(what);
      // A label the page ties to the field comes first, then its page
      // label; then what the field itself shows (its placeholder or title),
      // with the words a person reads beside it kept as its description;
      // the words beside it name it only when it says nothing itself. Live,
      // a neighbour's words ("Location not set", a divider's "OR") named a
      // search box and a location box, and the steps never found them.
      const near = nearby(element, checkable);
      const own = squash(element.getAttribute('placeholder')) || title;
      const name = squash(labels) || aria || own || near
        || (tag(element) === 'input' && !['text', 'search', 'password'].includes(element.type) ? squash(element.value) : '');
      const extra = [aria, near].find((said) => said && said !== name) || '';
      return { name: clip(name, limits.name), description: clip(extra, limits.name) };
    }
    const text = withoutGlyphs(element, ownText(element));
    if (text) {
      // A calendar's day is described by its date unless what the page says
      // of it already names a month, which stands alone: a number and a
      // fare name none, and anything more it says ("Sold out") follows the
      // date.
      const dated = calendarDays.get(element);
      const own = aria || innerLabel(element, text);
      const said = !dated || (own && MONTH_WORD.test(own)) ? own || dated
        : [dated, own].filter((part) => part && !text.includes(part)).join(', ');
      const description = said && said !== text && !text.includes(said) ? clip(said, limits.name) : '';
      return { name: clip(text, limits.name), description };
    }
    const pictured = [...element.querySelectorAll('img[alt], svg title')]
      .map((picture) => squash(picture.getAttribute('alt') || picture.textContent))
      .find(Boolean);
    const name = aria || title || pictured || '';
    if (name) return { name: clip(name, limits.name), description: '' };
    const icon = iconWords(element);
    if (icon) return { name: icon, description: 'an icon' };
    const step = stepperWord(element);
    if (step) return { name: step, description: 'an icon beside a count' };
    // A picture link with no words: where it leads is all there is to go on.
    const href = tag(element) === 'a' && element.getAttribute('href');
    if (href) {
      try {
        const path = new URL(href, location.href).pathname.split('/').filter(Boolean).pop() || '';
        const read = squash(decodeURIComponent(path).replace(/\.[a-z]+$/i, '').replace(/[-_]+/g, ' '));
        if (read) return { name: '', description: clip(`leads to ${read}`, limits.name) };
      } catch (error) { /* an unreadable address names nothing */ }
    }
    return { name: '', description: '' };
  };

  const CARDS = { li: 'listitem', tr: 'row', article: 'article' };
  const CARD_ROLES = ['listitem', 'row', 'article', 'option', 'treeitem', 'gridcell'];
  const LANDMARKS = {
    header: 'banner', nav: 'navigation', main: 'main', footer: 'contentinfo',
    aside: 'complementary', form: 'form', fieldset: 'group', section: 'region',
  };
  const GROUP_ROLES = [
    'banner', 'navigation', 'main', 'contentinfo', 'complementary', 'form', 'search', 'region',
    'group', 'listbox', 'menu', 'menubar', 'tablist', 'radiogroup', 'grid', 'table', 'tree',
    'list', 'toolbar', 'tabpanel',
  ];
  const heading = (element) => {
    const found = element.querySelector('h1, h2, h3, h4, h5, h6, [role="heading"], legend');
    return found && shown(found) ? clip(found.innerText, 60) : '';
  };
  const labelOf = (element) => {
    const scope = element.getRootNode();
    return clip(
      element.getAttribute('aria-label')
        || byIds(element.getAttribute('aria-labelledby'), scope.getElementById ? scope : document),
      60,
    );
  };

  // An element that floats above the page: a dialog, or a fixed layer that
  // is not the page's own header.
  const layer = (element) => {
    const name = tag(element);
    const claimed = role(element);
    if (name === 'dialog' && element.open) return 'dialog';
    if (claimed === 'dialog' || claimed === 'alertdialog') return claimed;
    if (element.getAttribute('aria-modal') === 'true') return 'dialog';
    if (style(element).position !== 'fixed' || name === 'header' || name === 'nav') return null;
    const rect = box(element);
    if (rect.width * rect.height < width * height * 0.05 || !shown(element)) return null;
    if (rect.top <= 0 && rect.height < height * 0.25 && element.querySelector('nav, a[href]')) return null;
    return rect.width * rect.height >= width * height * 0.3 ? 'dialog' : 'popover';
  };

  // Noise: what a person skips or never sees. Ads — frames and links to ad
  // servers, blocks the page names as ads, blocks labelled "Advertisement"
  // or "Sponsored", tracking pixels — and content the page hides: marked
  // `aria-hidden` or `inert`, or clipped out of sight for screen readers.
  // A noise block is left out whole; `denoised` counts the blocks that held
  // something sight would otherwise have returned.
  const AD_HOSTS = [
    'doubleclick.net', 'googlesyndication.com', 'googleadservices.com', 'amazon-adsystem.com',
    'taboola.com', 'outbrain.com', 'adnxs.com', 'moatads.com', 'pubmatic.com',
    'rubiconproject.com', 'scorecardresearch.com',
  ];
  const AD_HOST_NAMES = /(^|\.)(adservice\.google|criteo)\.[a-z]{2,}(\.[a-z]{2,})?$/;
  // The words of a class or id, split at `-` and `_` only: `ad`, not the
  // `ad` in `header`, `shadow`, `download`, or `adults`, nor in a generated
  // class such as `css-1ad4k9`; `AdSlot` reads as `adslot`. A short word
  // counts alone (`ads`) or beside a real word (`top-ad`, `div-gpt-ad-1`),
  // in one case: Google's generated `gb_Ad` and `gb_ad` are not ads.
  const AD_SHORT = /^(ad|ads|dfp|AD|ADS|DFP)$/;
  const AD_WORD = /^(adsbygoogle|ad(slot|unit|box|zone|space|container|wrapper|banner|frame|holder|placement)s?|advert\w*|sponsor\w*)$/i;
  const AD_LABEL = /^(advertisement|sponsored|ad)$/i;
  // Words that mark a cookie, consent, or newsletter banner, which the
  // obstacle loop must see to close: no ad rule ever drops one.
  const BOILERPLATE = /cookie|consent|gdpr|privacy|newsletter|subscri/i;
  const adHost = (address) => {
    if (!address) return false;
    let host = '';
    try { host = new URL(address, location.href).hostname.toLowerCase(); } catch (error) { return false; }
    return AD_HOSTS.some((name) => host === name || host.endsWith(`.${name}`)) || AD_HOST_NAMES.test(host);
  };
  const classText = (element) => (typeof element.className === 'string' ? element.className
    : (element.className && element.className.baseVal) || '');
  const adToken = (token) => {
    const words = token.split(/[_-]+/).filter(Boolean);
    if (words.some((word) => AD_WORD.test(word))) return true;
    if (!words.some((word) => AD_SHORT.test(word))) return false;
    return words.length === 1 || words.some((word) => /^[a-z]{3,}$/i.test(word) && !AD_SHORT.test(word));
  };
  const adWords = (element) => `${classText(element)} ${element.id || ''}`
    .split(/\s+/)
    .some(adToken);
  const boilerplate = (element) => BOILERPLATE.test(
    `${classText(element)} ${element.id || ''} ${element.getAttribute('aria-label') || ''} `
    + (element.textContent || '').slice(0, 600),
  );
  // Whether the element floats above the page, or sits in something that
  // does: an ad in front is an obstacle a person must close, not noise.
  const floats = (element) => {
    for (let parent = element; parent && parent !== document.documentElement; parent = parent.parentElement) {
      if (layer(parent)) return true;
    }
    return false;
  };
  const pixel = (element) => {
    if (tag(element) !== 'img' || !element.complete || element.naturalWidth < 1) return false;
    const rect = box(element);
    return rect.width <= 1 && rect.height <= 1 && element.naturalWidth <= 1 && element.naturalHeight <= 1;
  };
  const advert = (element) => {
    const address = element.getAttribute('src') || (tag(element) === 'a' && element.getAttribute('href'));
    const marked = adHost(address) || pixel(element) || adWords(element)
      || element.hasAttribute('data-ad-slot') || element.hasAttribute('data-ad-client')
      || element.hasAttribute('data-google-query-id');
    return marked && !boilerplate(element) && !floats(element);
  };
  // Clipped out of sight but still laid out: the visually-hidden text
  // pages keep for screen readers.
  const clipped = (element) => {
    const computed = style(element);
    if (computed.position !== 'absolute' && computed.position !== 'fixed') return false;
    if (computed.clip === 'rect(0px, 0px, 0px, 0px)' || computed.clipPath === 'inset(50%)') return true;
    const rect = box(element);
    return rect.width <= 1 && rect.height <= 1 && computed.overflow === 'hidden';
  };
  // What a hit test at the element's middle lands on.
  const hitAt = (element) => {
    const rect = box(element);
    const x = Math.min(Math.max((rect.left + rect.right) / 2, 0), width - 1);
    const y = Math.min(Math.max((rect.top + rect.bottom) / 2, 0), height - 1);
    return document.elementFromPoint(x, y);
  };
  const inFront = (element) => {
    const hit = hitAt(element);
    return hit === element || Boolean(hit && element.contains(hit));
  };
  // Whether a person sees what the page marks `aria-hidden`: pages mark
  // plenty they draw — a custom list's shown label, a pill below the fold,
  // a page a modal library forgot to unmark. Only what is slid out
  // sideways (a carousel's clones) or sits behind something in the
  // viewport (the page behind a dialog) is out of their sight.
  const plainlySeen = (element) => {
    const rect = box(element);
    if (rect.width < 1 || rect.height < 1) return true;
    if (rect.right <= 0 || rect.left >= width) return false;
    if (rect.bottom <= 0 || rect.top >= height) return true;
    if (inFront(element)) return true;
    // A hit passes through what takes no pointer events, so landing on what
    // holds such an element means nothing is drawn over it. Live, a seat
    // table drew each seat's number and status in `aria-hidden` cells that
    // take no pointer events, and every row in view lost them. Only an
    // element that turns pointer events off itself counts, and never a hit
    // on the page's root: a modal library turns them off for the whole body
    // while it hides the page behind its dialog.
    const own = style(element).pointerEvents === 'none'
      && !(element.parentElement && style(element.parentElement).pointerEvents === 'none');
    const hit = hitAt(element);
    return Boolean(own && hit && hit !== document.documentElement && hit !== document.body
      && hit.contains(element));
  };
  // Blocks labelled as ads, found once up front: the label and the nearest
  // block around it that holds the ad, but never a landmark, a form, a
  // dialog, a banner a person must answer, or much of the page.
  const labelledAds = new Set();
  const HOLDS = `${NESTED}, img, iframe, video, picture, canvas`;
  const enclosable = (element) => element !== base && element !== document.body
    && element.parentElement !== null
    && !element.matches(`main, form, header, nav, footer, [role="main"], [role="form"], [role="navigation"], [role="banner"], [role="contentinfo"], ${MODAL_SELECTOR}`)
    && !element.querySelector(`main, form, input:not([type="hidden"]), select, textarea, ${MODAL_SELECTOR}`)
    && box(element).width * box(element).height <= width * height * 0.4
    && !boilerplate(element) && !floats(element);
  const findLabelledAds = () => {
    const walker = document.createTreeWalker(base, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const said = squash(node.data);
      const label = node.parentElement;
      if (!label || !AD_LABEL.test(said) || squash(label.innerText) !== said || !shown(label)) continue;
      // A control named "Ad" by itself is a control, not a label.
      const control = label.closest(NESTED);
      if (control && squash(control.innerText) === said) continue;
      let block = null;
      for (let parent = label; parent && enclosable(parent); parent = parent.parentElement) {
        if (squash(parent.textContent) !== said || parent.querySelector(HOLDS)) {
          block = parent;
          break;
        }
      }
      if (block) labelledAds.add(block);
    }
  };
  // What kind of noise the element itself is, or null.
  const noise = (element) => {
    if (element === base) return null;
    if (element.hasAttribute('inert')) return 'hidden';
    if (element.getAttribute('aria-hidden') === 'true' && !plainlySeen(element)) return 'hidden';
    if (labelledAds.has(element) || advert(element)) return 'ads';
    return clipped(element) ? 'hidden' : null;
  };
  const noiseRoots = new Map();
  const noiseKinds = new Map();
  // The outermost noise block the element is in (or is), or null.
  const noiseRoot = (element) => {
    if (noiseRoots.has(element)) return noiseRoots.get(element);
    let root = null;
    if (element !== base && base.contains(element)) {
      root = element.parentElement && noiseRoot(element.parentElement);
      const own = root ? null : noise(element);
      if (own) {
        root = element;
        noiseKinds.set(element, own);
      }
    }
    noiseRoots.set(element, root);
    return root;
  };
  const denoised = { ads: 0, empty: 0, hidden: 0 };
  const tallied = new Set();
  const tally = (root) => {
    if (tallied.has(root)) return;
    tallied.add(root);
    denoised[noiseKinds.get(root)] += 1;
  };

  // Result cards a page draws as plain boxes: three or more siblings of
  // one tag and class (or one more of a kind already found), each holding
  // a link or button, a line of words, and links to one place at most two
  // ways (a picture and a title). Live, a store's product grid was all
  // `div`s, so no list of products showed, and a pick took a row of
  // carousel dots for the results; and a grid laid out in rows of four
  // read each row as one card, whose first link was another product. The
  // card's place among all cards of its kind on the page, counted in page
  // order, so the rows' cards make one list; 0 when it is not one.
  const siblingKinds = new Map();
  const kindCounts = new Map();
  const kindOf = (element) => `${element.tagName} ${classText(element).trim()}`;
  const repeatedCard = (element) => {
    const parent = element.parentElement;
    if (!parent || !classText(element).trim()) return 0;
    let kinds = siblingKinds.get(parent);
    if (!kinds) {
      kinds = new Map();
      for (const child of parent.children) kinds.set(kindOf(child), (kinds.get(kindOf(child)) || 0) + 1);
      siblingKinds.set(parent, kinds);
    }
    const kind = kindOf(element);
    if ((kinds.get(kind) || 0) < 3 && !kindCounts.has(kind)) return 0;
    if (!element.querySelector('a[href], button, [role="button"], [role="link"]')) return 0;
    const places = new Set([...element.querySelectorAll('a[href]')].map((link) => link.getAttribute('href')));
    if (places.size > 2) return 0;
    // A card says something in words: a carousel's numbered dots ("1 2 3
    // … 22") are long enough, but name nothing (live, a pick took them).
    const said = squash(element.innerText);
    if (said.length < 20 || !/\p{L}{3}/u.test(said)) return 0;
    // A row that holds cards already counted is their row, not a card.
    if ([...element.querySelectorAll('[class]')].some((inner) => kindCounts.has(kindOf(inner)))) return 0;
    const ordinal = (kindCounts.get(kind) || 0) + 1;
    kindCounts.set(kind, ordinal);
    return ordinal;
  };

  // A table row's own words: what its cells holding no control say, as a
  // person reads across a row to its button. Live, a seat table's rows
  // read `row #1`, and the "Select" of a seat whose status cell said
  // "Handicapped" was pressed for an available one.
  const rowWords = (element) => clip([...element.children]
    .filter((cell) => !cell.matches(NESTED) && !cell.querySelector(NESTED))
    .map((cell) => cell.textContent)
    .join(' '), 60);

  const containers = new Map();
  const unnamed = new Map();
  // The container label a person would see `element` as, or null.
  const container = (element) => {
    if (containers.has(element)) return containers.get(element);
    let label = null;
    let repeated = 0;
    const name = tag(element);
    const claimed = role(element);
    const floating = layer(element);
    if (floating) {
      const named = labelOf(element) || heading(element);
      label = named ? `${floating} ${JSON.stringify(named)}` : floating;
    } else {
      const card = CARD_ROLES.includes(claimed) ? claimed : (!claimed && CARDS[name]);
      if (card) {
        const parent = element.parentElement;
        let ordinal = 1;
        for (let sibling = element.previousElementSibling; sibling; sibling = sibling.previousElementSibling) {
          if (sibling.tagName === element.tagName && role(sibling) === claimed) ordinal += 1;
        }
        const named = labelOf(element) || (card === 'row' ? rowWords(element) : '');
        label = named ? `${card} ${JSON.stringify(named)} #${ordinal}` : `${card} #${ordinal}`;
        if (!parent) label = null;
      } else if (!claimed && !LANDMARKS[name] && !['ul', 'ol'].includes(name)
        && (repeated = repeatedCard(element))) {
        const named = labelOf(element);
        label = named ? `listitem ${JSON.stringify(named)} #${repeated}` : `listitem #${repeated}`;
      } else {
        const group = GROUP_ROLES.includes(claimed) ? claimed
          : (LANDMARKS[name] || (['ul', 'ol'].includes(name) ? 'list' : null));
        if (group) {
          const named = labelOf(element) || (['region', 'group', 'form', 'dialog'].includes(group) ? heading(element) : '');
          if (group === 'region' && !named) label = null;
          else if (named) label = `${group} ${JSON.stringify(named)}`;
          else {
            // Unnamed lists are told apart by their place on the page, so
            // two lists' first cards are not read as one card.
            const count = (unnamed.get(group) || 0) + 1;
            unnamed.set(group, count);
            label = count === 1 ? group : `${group} ${count}`;
          }
        }
      }
    }
    containers.set(element, label);
    return label;
  };

  const pathOf = (element) => {
    const labels = [];
    for (let parent = element.parentElement; parent && parent !== document.documentElement; parent = parent.parentElement) {
      const label = container(parent);
      if (label) labels.push(label);
    }
    return labels.reverse();
  };

  // What is on top at the element's middle: itself, something inside it,
  // or something it sits in; anything else covers it.
  const covered = (element) => {
    const rect = box(element);
    const x = (rect.left + rect.right) / 2;
    const y = (rect.top + rect.bottom) / 2;
    if (x < 0 || y < 0 || x > width || y > height) return false;
    const hit = document.elementFromPoint(x, y);
    if (!hit || element.contains(hit) || hit.contains(element)) return false;
    const input = standIn(element);
    if (input && (hit === input || input.contains(hit))) return false;
    // A result card's own text laid over its link: clicking there is
    // clicking the card, as a person would.
    const card = element.closest(CARD_SELECTOR);
    if (card && card.contains(hit) && !hit.closest(MODAL_SELECTOR)) return false;
    return !(element.labels && [...element.labels].some((label) => label.contains(hit)));
  };

  const offscreen = (element) => {
    const rect = box(element);
    return rect.bottom <= 0 || rect.top >= height || rect.right <= 0 || rect.left >= width;
  };

  // The nearest container that scrolls its content (a popover's list), or
  // null. Each container is looked at once. An element fixed to the window
  // moves with no container, and none of them clips it: a fixed pop-up
  // drawn from inside a scrolling list shows wherever it is placed.
  const scrollers = new Map();
  const scrollerOf = (element) => {
    if (style(element).position === 'fixed') return null;
    const parent = element.parentElement;
    if (!parent || parent === document.body || parent === document.documentElement) return null;
    if (scrollers.has(parent)) return scrollers.get(parent);
    const overflow = style(parent);
    const scrolls = (/(auto|scroll)/.test(overflow.overflowY) && parent.scrollHeight > parent.clientHeight)
      || (/(auto|scroll)/.test(overflow.overflowX) && parent.scrollWidth > parent.clientWidth);
    const found = scrolls ? parent : scrollerOf(parent);
    scrollers.set(parent, found);
    return found;
  };
  // Whether the element's middle is scrolled out of its container's view:
  // what shows at that point is the container's neighbour or the page,
  // neither of which covers the element, and a press scrolls it back. Live,
  // a popover's airport rows below its list's fold read as covered, ranked
  // last, and were never offered.
  const scrolledAway = (element) => {
    const scroller = scrollerOf(element);
    if (!scroller) return false;
    const rect = box(element);
    const view = scroller.getBoundingClientRect();
    const x = (rect.left + rect.right) / 2;
    const y = (rect.top + rect.bottom) / 2;
    return x < view.left || x > view.right || y < view.top || y > view.bottom;
  };

  const statesOf = (element, what) => {
    const states = [];
    const input = standIn(element) || element;
    const aria = (name) => element.getAttribute(`aria-${name}`);
    if (input.checked === true || aria('checked') === 'true' || aria('pressed') === 'true') states.push('checked');
    if (aria('expanded') === 'true' || (tag(element) === 'summary' && element.parentElement && element.parentElement.open)) {
      states.push('expanded');
    }
    if (aria('selected') === 'true' || (aria('current') && aria('current') !== 'false')
      || (!states.includes('checked') && classChosen(element))) states.push('selected');
    if (input.required === true || aria('required') === 'true') states.push('required');
    if (offscreen(element) || scrolledAway(element)) states.push('offscreen');
    else if (covered(element)) states.push('covered');
    return states;
  };

  const valueOf = (element, what) => {
    const name = tag(element);
    if (name === 'select') {
      const chosen = element.selectedOptions && element.selectedOptions[0];
      return chosen ? squash(chosen.textContent) : '';
    }
    if (what === 'textbox' || what === 'searchbox') {
      if (name === 'input' && element.type === 'password') return '';
      return name === 'input' || name === 'textarea' ? element.value : element.innerText;
    }
    if (what === 'slider') return String(element.value);
    return '';
  };

  let next = Number(window.__tinycomputerSeen || 1);
  const mark = (element) => {
    let id = element.getAttribute('data-tc-seen');
    if (!id) {
      id = String(next);
      next += 1;
      element.setAttribute('data-tc-seen', id);
    }
    return id;
  };

  findLabelledAds();
  collectWords();
  findCalendars();
  const nodes = [];
  const controls = new Set();
  const seen = [];
  // Intersection over union of two boxes.
  const overlap = (a, b) => {
    const across = Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left));
    const down = Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top));
    const shared = across * down;
    const union = a.width * a.height + b.width * b.height - shared;
    return union > 0 ? shared / union : 0;
  };
  const home = (element) => element.closest('label') || (element.labels && element.labels[0]) || null;
  // Whether `element` is what a person sees as the control `other` already
  // is: drawn in the same box, drawn inside it with the same words, or the
  // same kind of control in the same label (a page's own radio drawn beside
  // the real one).
  const same = (other, element, what) => {
    const related = other.element.contains(element) || element.contains(other.element)
      || other.element.parentElement === element.parentElement;
    if (related && overlap(box(other.element), box(element)) >= 0.6) return true;
    if (other.element.contains(element)
      && squash(other.element.innerText) === squash(element.innerText)) return true;
    return other.record.role === what && home(element) !== null && home(element) === home(other.element);
  };
  // A native button or link inside a control a page only claims (a table
  // cell with `role="gridcell"`, a row a script makes pressable) is what a
  // press must reach: the wrapper's middle can be bare cell. Live, a seat
  // table's "Select" buttons sat at their cells' left edge, and six presses
  // at the cells' middles selected nothing.
  const NATIVE_PRESS = 'button, a[href], summary, input[type="button"], input[type="submit"]';
  // Never a control that says whether it is chosen (a tab, a radio, an
  // option): what selects it checks that state on the record's element, and
  // the inner one never carries it, so it would be pressed twice.
  const CHOOSING_ROLES = ['tab', 'radio', 'option', 'checkbox', 'switch', 'menuitemradio',
    'menuitemcheckbox', 'treeitem'];
  const pressedInside = (wrapper, element) => wrapper !== element && wrapper.contains(element)
    && element.matches(NATIVE_PRESS) && !wrapper.matches(`${NATIVE_PRESS}, input, select, textarea, label`)
    && !CHOOSING_ROLES.includes(role(wrapper));
  // Points `twin`'s record at `element`, the control inside it, keeping
  // what the wrapper says of itself (selected, checked) beside where the
  // inner control is drawn and whether something covers it.
  const aimAt = (twin, element) => {
    const placed = ['offscreen', 'covered'];
    const own = twin.record.states.filter((state) => !placed.includes(state));
    const inner = statesOf(element, twin.record.role);
    twin.record.states = [...new Set([...own, ...inner])];
    twin.record.id = mark(element);
    twin.record.box = [box(element).x, box(element).y, box(element).width, box(element).height]
      .map(Math.round);
    twin.element = element;
  };
  let unreachable = 0;
  // Whether a shadow root's host shows controls, which a selector from the
  // page cannot address. A host drawn as `display: contents` has no box of
  // its own: live, a consent banner's host had none, and its buttons went
  // unread while the banner lay over the add-to-cart button. Its controls'
  // own boxes then say whether it shows.
  const showsShadowControls = (host) => {
    const controls = host.shadowRoot.querySelectorAll('a[href], button, input, select, textarea, [role], [tabindex]');
    return controls.length > 0 && (shown(host) || [...controls].some(shown));
  };
  // The shadow roots that show controls, each as its host's ref, which a
  // selector can address, and the label of the layer it draws, if any: the
  // controls the tree reads under the host keep the place they show in.
  // Live, a consent banner was a fixed layer over the page. Only the first
  // is labelled: with two, the tree reads the whole page instead.
  const shadows = [];
  // The first host handed to the tree. The tree reads under it the host
  // and what the page puts in its slots as well, so sight reads neither.
  let handed = null;
  const firstWords = (element) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const text = squash(node.data);
      if (text.split(' ').length >= 3 && node.parentElement && shown(node.parentElement)) return clip(text, 60);
    }
    return '';
  };
  const shadowLabel = (host) => {
    for (const element of host.shadowRoot.querySelectorAll('*')) {
      const floating = layer(element);
      if (!floating || !shown(element)) continue;
      const named = labelOf(element) || heading(element) || firstWords(element);
      return named ? `${floating} ${JSON.stringify(named)}` : floating;
    }
    return null;
  };
  let texts = 0;
  const insideControl = (element) => {
    for (let parent = element.parentElement; parent; parent = parent.parentElement) {
      if (controls.has(parent) && !scriptedOnly.has(parent)) return true;
    }
    return false;
  };
  const NATIVE = ['a', 'button', 'summary', 'input', 'select', 'textarea', 'label', 'img', 'svg'];
  const blank = (element) => !NATIVE.includes(tag(element))
    && !ROLES.includes(role(element)) && !TEXT_ROLES.includes(role(element))
    && !element.querySelector(`${NESTED}, img, svg, picture, canvas, video`);

  // A native dropdown, and a text box's list of suggestions (`<datalist>`),
  // show their choices in a menu the browser draws outside the page, where
  // nothing reads or presses them. Each choice is offered as an option
  // inside its control, drawn in the control's box, and pressing one sets
  // the control's value (`native_select.rs`).
  const choicesOf = (element) => {
    if (tag(element) === 'select') return element.multiple ? [] : [...element.options];
    const list = tag(element) === 'input' ? element.list : null;
    if (!list || element.readOnly) return [];
    // Suggestions two boxes share are offered under the one being typed
    // in, so that pressing one names a single box to fill.
    const users = [...document.querySelectorAll('input[list]')].filter((other) => other.list === list);
    if (users.length > 1 && document.activeElement !== element) return [];
    return [...list.querySelectorAll('option')];
  };
  const offerChoices = (element, record) => {
    const suggested = tag(element) !== 'select';
    const inside = [
      ...record.path,
      `listbox ${JSON.stringify(record.name || (suggested ? 'suggestions' : 'dropdown'))}`,
    ];
    let listed = 0;
    for (const option of choicesOf(element)) {
      if (listed >= OPTIONS_PER_DROPDOWN) break;
      const group = option.parentElement;
      if (option.disabled || (group && tag(group) === 'optgroup' && group.disabled)) continue;
      // A suggestion is named by what it fills in, a dropdown's choice by
      // what it shows.
      const said = squash(option.label);
      const label = suggested ? squash(option.value) || said : said || squash(option.textContent);
      if (!label) continue;
      listed += 1;
      if (suggested) option.setAttribute('data-tc-for', record.id);
      nodes.push({
        id: mark(option),
        role: 'option',
        name: clip(label, limits.name),
        description: suggested && said && said !== label ? clip(said, limits.name) : '',
        value: '',
        states: (suggested ? element.value === option.value : option.selected) ? ['selected'] : [],
        box: record.box,
        path: inside,
      });
    }
  };

  const walker = document.createTreeWalker(base, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode: (node) => {
      if (node.nodeType === Node.ELEMENT_NODE
        && ['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD'].includes(node.tagName)) {
        return NodeFilter.FILTER_REJECT;
      }
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  let lastText = null;
  for (let node = base; node; node = walker.nextNode()) {
    if (handed && handed.contains(node)) continue;
    if (node.nodeType === Node.TEXT_NODE) {
      const parent = node.parentElement;
      if (!parent || !squash(node.data) || texts >= limits.texts) continue;
      if (lastText && lastText.contains(parent)) continue;
      if (controls.has(parent) || insideControl(parent) || insideText(parent) || !shown(parent)) continue;
      const rect = box(parent);
      if (rect.bottom < -height || rect.top > 2 * height) continue;
      const dropped = noiseRoot(parent);
      if (dropped) {
        tally(dropped);
        continue;
      }
      lastText = parent;
      texts += 1;
      nodes.push({ text: clip(shownWords(parent) || node.data, limits.text), path: pathOf(parent) });
      continue;
    }
    const element = node;
    const dropped = noiseRoot(element);
    // An ad's own frame or picture is an ad whether or not it shows words.
    if (dropped === element && ['iframe', 'img'].includes(tag(element))
      && element.getClientRects().length > 0 && noiseKinds.get(element) === 'ads') {
      tally(element);
    }
    if (element.shadowRoot && showsShadowControls(element)) {
      if (dropped) tally(dropped);
      else {
        shadows.push({ id: mark(element), label: shadows.length ? null : shadowLabel(element) });
        if (!handed) {
          handed = element;
          continue;
        }
      }
    }
    if (tag(element) === 'iframe' && shown(element) && !offscreen(element) && inFront(element)) {
      const rect = box(element);
      if (rect.width * rect.height >= width * height * 0.2) {
        if (dropped) tally(dropped);
        else unreachable += 1;
      }
    }
    // A big drawn area (a canvas, or an svg picture without words) holds
    // no controls to read: say so, so a seat map or a chart drawn there is
    // not taken for an empty page. Such a page often offers an accessible
    // alternative, which the flow guide tells a planner to open.
    const drawn = tag(element) === 'canvas'
      || (tag(element) === 'svg' && !element.querySelector('text, a, [role]'));
    if (drawn && texts < limits.texts && shown(element) && !offscreen(element)) {
      const rect = box(element);
      if (rect.width * rect.height >= width * height * 0.15) {
        texts += 1;
        nodes.push({
          text: `a drawn ${tag(element) === 'canvas' ? 'canvas' : 'picture'} with no controls to press, ${Math.round(rect.width)}x${Math.round(rect.height)}`,
          path: pathOf(element),
        });
      }
    }
    if (controls.size >= limits.controls || disabled(element)) continue;
    const what = kind(element, insideControl(element));
    if (!what || !shown(element)) continue;
    // A box to type in is never part of something pressed: a button or link
    // that holds one is a panel (a popover with its own search box), and the
    // rows it lists are read as controls of their own. Read as one button,
    // its name strings every row together, and a press lands on whatever row
    // sits at its middle.
    if ((what === 'button' || what === 'link') && holdsField(element)) continue;
    if (tag(element) === 'input' && (element.type === 'checkbox' || element.type === 'radio')) {
      // Drawn by its label instead: the label stands in for it.
      if ([...(element.labels || [])].some((label) => standIn(label) === element)) continue;
    }
    if (dropped) {
      tally(dropped);
      continue;
    }
    const named = naming(element, what);
    const name = monthTurn(element, named.name);
    const { description } = named;
    // A blank box that is clickable only by its cursor, tab stop, or click
    // handler: no words, no name, no picture, nothing inside to act on.
    if (!name && !description && blank(element)) {
      denoised.empty += 1;
      continue;
    }
    // Two elements drawn as one box are one control to a person: the one
    // that takes text, or else the first, with the other's words kept.
    const twin = seen.find((other) => same(other, element, what));
    if (twin) {
      const takes = what === 'textbox' || what === 'searchbox';
      const twinTakes = twin.record.role === 'textbox' || twin.record.role === 'searchbox';
      if (!takes || twinTakes) {
        if (name && (!twin.record.name || twin.record.description === 'an icon')) {
          twin.record.name = name;
          twin.record.description = description;
        }
        else if (!twin.record.description && name && name !== twin.record.name) twin.record.description = name;
        if (pressedInside(twin.element, element)) aimAt(twin, element);
        controls.add(element);
        continue;
      }
      nodes.splice(nodes.indexOf(twin.record), 1);
      seen.splice(seen.indexOf(twin), 1);
    }
    controls.add(element);
    const record = {
      id: mark(element),
      role: what,
      name,
      description,
      value: valueOf(element, what),
      states: statesOf(element, what),
      box: [box(element).x, box(element).y, box(element).width, box(element).height].map(Math.round),
      path: pathOf(element),
    };
    nodes.push(record);
    seen.push({ element, record });
    offerChoices(element, record);
  }
  // A control repeated on every card ("ADD" on each product), kept inside
  // the card that is a control itself, says nothing of which card it acts
  // on: it is described by that card's name. Live, "add the first Maggi"
  // pressed the first card's "ADD", on a ramen above the Maggi.
  const recordOf = new Map(seen.map(({ element, record }) => [element, record]));
  const copies = new Map();
  for (const { record } of seen) {
    const key = `${record.role}\u0000${record.name}`;
    copies.set(key, (copies.get(key) || 0) + 1);
  }
  for (const { element, record } of seen) {
    if (record.description || !record.name
      || copies.get(`${record.role}\u0000${record.name}`) < 2) continue;
    for (let parent = element.parentElement; parent; parent = parent.parentElement) {
      const card = recordOf.get(parent);
      if (!card) continue;
      if (card.name && card.name !== record.name) record.description = clip(`in ${card.name}`, limits.name);
      break;
    }
  }
  window.__tinycomputerSeen = next;

  const middle = document.elementFromPoint(width / 2, height / 2);
  let surface = 'window';
  for (let parent = middle; parent; parent = parent.parentElement) {
    const floating = layer(parent);
    if (floating === 'alertdialog') { surface = 'alert'; break; }
    if (floating === 'dialog') { surface = 'sheet'; break; }
  }
  return { ok: true, title: document.title, surface, unreachable, shadows, nodes, denoised };
})

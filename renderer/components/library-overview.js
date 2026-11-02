// LibraryOverview — the desktop's Overview view (nav bar, after Categories).
//
// Four cards over the scanned library (renderer/js/library-overview.js):
//   • Capacity: videos against LIBRARY_MAX_VIDEOS (the cap the scouts stop
//     downloading at), size on disk, per source
//   • Ratings and favourites: counts, the triage buckets (Keep / Review), a
//     star histogram and every rated or favourite video with its stars
//   • Favourite artists: star per artist folder, add a name, the favourite
//     artist bonus
//   • Tag scores: every tag with its video count and a −/+ score, the
//     built-in weights and Recalculate scores
//
// Artists, the bonus, tag scores and Recalculate go through `applyEdit`, the
// app's library-edit applier (library-edits.js), so a desktop change is
// stored, scored and sent to the backend exactly like the same change from
// the web remote. Ratings use the data service directly, like the cards.
// The view repaints when those settings change from anywhere (the phone, the
// headset, a scan) while it is shown.

import { getCurrentLocale, t } from '../js/i18n.js';
import { eventBus } from '../js/event-bus.js';
import { icon, Star, Minus, Plus, RefreshCw } from '../js/icons.js';
import { createRatingControl } from '../js/rating-stars.js';
import { effectiveTagScores, tagScoreInfo } from '../js/preference-score.js';
import { favouriteScoreOf } from '../js/favourite-artists.js';
import { RANKING } from '../js/content-preferences.js';
import { showToast } from '../js/toast.js';
import {
  artistSummary, capacitySummary, formatBytes, formatTagScore, ratingSummary, stepTagScore, tagSummary,
} from '../js/library-overview.js';

// Built-in weights in the web remote's order (backend/web-remote/tag-scores.js).
const RULE_ORDER = ['multiAxis', 'vr', 'artistInLibrary', 'favouriteArtist', 'likes', 'hentaiOnly'];

const fold = (s) => String(s || '').normalize('NFC').trim().toLocaleLowerCase();
const titleOf = (p) => (String(p).split(/[\\/]/).pop() || '').replace(/\.[^/.]+$/, '');

// Settings whose change repaints a card.
const CARD_OF_SETTING = {
  'library.tagScores': ['tags'],
  'library.scriptTags': ['tags'],
  'library.favouriteArtists': ['artists'],
  'library.favouriteArtistScore': ['artists', 'tags'],
  'library.sources': ['capacity', 'ratings', 'artists', 'tags'],
};

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export class LibraryOverview {
  /**
   * @param {object} o
   * @param {object} o.settings DataService
   * @param {object} o.library Library component (scanned videos)
   * @param {(edit: object) => Promise<void>} o.applyEdit apply a library edit
   * @param {(filter: string) => void} o.onShowLibrary open the library with a rating filter
   * @param {(path: string) => void} o.onPlayVideo play a library video
   */
  constructor({ settings, library, applyEdit, onShowLibrary, onPlayVideo }) {
    this._settings = settings;
    this._library = library;
    this._applyEdit = applyEdit;
    this._onShowLibrary = onShowLibrary;
    this._onPlayVideo = onPlayVideo;
    this._container = null;
    this._cards = {};
    this._tagFilter = '';
    this._artistFilter = '';
    this._tagUnsent = new Map();      // tag → score being applied
    this._artistUnsent = new Map();   // folded name → favourite being applied
    this._bonusUnsent = undefined;    // bonus being applied
    this._recalculating = false;
    this._repaintTimer = null;
    this._dirty = new Set();
    this._listening = false;
  }

  show(containerEl) {
    this._container = containerEl;
    this._listen();
    this._render();
    if (!this._library?._videos?.length) {
      this._library?.ensureScanned?.().then(() => this._repaint(['capacity', 'ratings', 'artists', 'tags'])).catch(() => {});
    }
  }

  hide() {
    if (this._container) this._container.replaceChildren();
    this._cards = {};
  }

  get _visible() {
    return !!this._container && !this._container.hidden && !!this._cards.capacity;
  }

  _listen() {
    if (this._listening) return;
    this._listening = true;
    eventBus.on('settings:changed', ({ path } = {}) => {
      const cards = CARD_OF_SETTING[path];
      if (cards) this._repaint(cards);
    });
    eventBus.on('rating:changed', () => this._repaint(['ratings']));
    eventBus.on('library:scanned', () => this._repaint(['capacity', 'ratings', 'artists', 'tags']));
    eventBus.on('language:changed', () => { if (this._visible) this._render(); });
  }

  /** Repaint `cards` soon (several changes in a row repaint once). */
  _repaint(cards) {
    for (const c of cards) this._dirty.add(c);
    clearTimeout(this._repaintTimer);
    this._repaintTimer = setTimeout(() => {
      const dirty = [...this._dirty];
      this._dirty.clear();
      if (!this._visible) return;
      for (const c of dirty) this._paintCard(c);
    }, 50);
  }

  _videos() {
    return this._library?._videos || [];
  }

  _videoPaths() {
    return this._videos().map((v) => v.path);
  }

  _render() {
    if (!this._container) return;
    const header = el('div', 'overview__header');
    header.appendChild(el('span', 'overview__title', t('overview.title')));
    const body = el('div', 'overview__body');
    this._cards = {};
    for (const id of ['capacity', 'ratings', 'artists', 'tags']) {
      const card = el('section', `overview__card overview__card--${id}`);
      card.dataset.card = id;
      this._cards[id] = card;
      body.appendChild(card);
    }
    this._container.replaceChildren(header, body);
    for (const id of Object.keys(this._cards)) this._paintCard(id);
  }

  _paintCard(id) {
    const card = this._cards[id];
    if (!card) return;
    if (id === 'capacity') this._paintCapacity(card);
    else if (id === 'ratings') this._paintRatings(card);
    else if (id === 'artists') this._paintArtists(card);
    else if (id === 'tags') this._paintTags(card);
  }

  _cardTitle(text, extra) {
    const h = el('div', 'overview__card-head');
    h.appendChild(el('h2', 'overview__card-title', text));
    if (extra) h.appendChild(extra);
    return h;
  }

  // --- Capacity --------------------------------------------------------------

  _paintCapacity(card) {
    const cap = capacitySummary(this._videos());
    const size = formatBytes(cap.bytes, getCurrentLocale());
    const head = this._cardTitle(t('overview.capacity.title'));

    const big = el('div', 'overview__capacity-figures');
    big.appendChild(el('span', 'overview__capacity-count', t('overview.capacity.videos', { count: cap.count, cap: cap.cap })));
    big.appendChild(el('span', `overview__capacity-percent overview__level--${cap.level}`, t('overview.capacity.percent', { percent: cap.percent })));

    const bar = el('div', `overview__bar overview__bar--capacity overview__level--${cap.level}`);
    bar.setAttribute('role', 'meter');
    bar.setAttribute('aria-valuemin', '0');
    bar.setAttribute('aria-valuemax', String(cap.cap));
    bar.setAttribute('aria-valuenow', String(cap.count));
    bar.setAttribute('aria-label', t('overview.capacity.title'));
    const fill = el('div', 'overview__bar-fill');
    fill.style.width = `${Math.min(100, cap.percent)}%`;
    bar.appendChild(fill);

    const facts = el('div', 'overview__capacity-facts');
    facts.appendChild(el('span', '', t('overview.capacity.size', { size })));
    facts.appendChild(el('span', '', cap.level === 'full' ? t('overview.capacity.full') : t('overview.capacity.free', { count: cap.free })));

    const table = el('div', 'overview__sources');
    for (const s of cap.sources) {
      const row = el('div', 'overview__source');
      row.appendChild(el('span', 'overview__source-name', s.name || '—'));
      const share = el('div', 'overview__bar overview__bar--source');
      const sFill = el('div', 'overview__bar-fill');
      sFill.style.width = `${cap.count ? (s.count / cap.count) * 100 : 0}%`;
      share.appendChild(sFill);
      row.appendChild(share);
      row.appendChild(el('span', 'overview__source-count', t('overview.videoCount', { count: s.count })));
      row.appendChild(el('span', 'overview__source-size', formatBytes(s.bytes, getCurrentLocale())));
      table.appendChild(row);
    }

    const hint = el('p', 'overview__hint', t('overview.capacity.hint', { cap: cap.cap }));
    card.replaceChildren(head, big, bar, facts, table, hint);
  }

  // --- Ratings ---------------------------------------------------------------

  _paintRatings(card) {
    const paths = this._videoPaths();
    const r = ratingSummary(this._settings.get('library.ratings') || {}, paths);
    const head = this._cardTitle(t('overview.ratings.title'));

    const tiles = el('div', 'overview__tiles');
    const tile = (label, value, filter, extraClass = '') => {
      const node = el(filter ? 'button' : 'div', `overview__tile ${extraClass}`.trim());
      if (filter) {
        node.type = 'button';
        node.title = t('overview.ratings.showInLibrary');
        node.addEventListener('click', () => this._onShowLibrary?.(filter));
      }
      node.append(el('span', 'overview__tile-value', String(value)), el('span', 'overview__tile-label', label));
      tiles.appendChild(node);
    };
    tile(t('rating.filterFavorites'), r.favourites, 'favorites', 'overview__tile--favourite');
    tile(t('overview.ratings.keep'), r.keep, 'top');
    tile(t('overview.ratings.review'), r.review, null);
    tile(t('rating.filterUnrated'), r.unrated, 'unrated');

    const histo = el('div', 'overview__histogram');
    const max = Math.max(1, ...r.stars.map((s) => s.count));
    for (const s of r.stars.filter((x) => Number.isInteger(x.rating) || x.count > 0)) {
      const row = el('div', 'overview__histo-row');
      const label = el('span', 'overview__histo-label');
      label.append(el('span', '', String(s.rating)), icon(Star, { width: 12, height: 12 }));
      const bar = el('div', 'overview__bar overview__bar--histo');
      const fill = el('div', 'overview__bar-fill');
      fill.style.width = `${(s.count / max) * 100}%`;
      bar.appendChild(fill);
      row.append(label, bar, el('span', 'overview__histo-count', String(s.count)));
      histo.appendChild(row);
    }

    const children = [head, tiles, histo];
    if (r.entries.length) {
      children.push(el('h3', 'overview__subtitle', t('overview.ratings.listTitle', { count: r.entries.length })));
      const list = el('div', 'overview__list overview__list--rated');
      for (const entry of r.entries) {
        const row = el('div', 'overview__row');
        const name = el('button', 'overview__row-name overview__row-name--link', titleOf(entry.path));
        name.type = 'button';
        name.title = entry.path;
        name.setAttribute('aria-label', t('overview.ratings.play', { name: titleOf(entry.path) }));
        name.addEventListener('click', () => this._onPlayVideo?.(entry.path));
        row.appendChild(name);
        row.appendChild(createRatingControl({
          rating: entry.rating,
          favorite: entry.favorite,
          size: 14,
          className: 'overview__rating',
          onRate: (rating) => this._settings.setRating(entry.path, { rating }),
          onFavorite: (favorite) => this._settings.setRating(entry.path, { favorite }),
        }));
        list.appendChild(row);
      }
      children.push(list);
    } else {
      children.push(el('p', 'overview__empty', t('overview.ratings.none')));
    }
    card.replaceChildren(...children);
  }

  // --- Favourite artists -----------------------------------------------------
  //
  // The card is built once per render; repaints update the count, the bonus
  // and the rows in place. Rows keep their order while the same artists are
  // listed, so a starred artist doesn't jump away under the pointer (as in
  // the web remote); the list is sorted again when an artist comes or goes.

  _artistRows() {
    return artistSummary(this._settings, this._videoPaths()).map((a) => {
      const key = fold(a.name);
      return this._artistUnsent.has(key) ? { ...a, favourite: this._artistUnsent.get(key), pending: true } : a;
    });
  }

  _displayedBonus() {
    return this._bonusUnsent ?? favouriteScoreOf(this._settings);
  }

  _paintArtists(card) {
    const rows = this._artistRows();
    this._artistRowsShown = rows;
    const ui = this._artistUi?.card === card ? this._artistUi : this._buildArtistsCard(card);
    ui.count.textContent = t('overview.artists.favouriteCount', { count: rows.filter((a) => a.favourite).length });
    const bonus = this._displayedBonus();
    this._updateScoreEditor(ui.bonus, { label: t('webRemote.artists.bonus'), score: bonus, pending: this._bonusUnsent !== undefined });
    ui.bonusDefault.textContent = bonus === RANKING.favouriteArtist
      ? t('webRemote.artists.bonusIsDefault')
      : t('webRemote.artists.bonusDefault', { score: formatTagScore(RANKING.favouriteArtist) });
    this._fillArtistList(ui.list, rows);
  }

  _buildArtistsCard(card) {
    const count = el('span', 'overview__card-count');
    const head = this._cardTitle(t('overview.artists.title'), count);
    const hint = el('p', 'overview__hint', t('webRemote.artists.hint'));

    const bonusRow = el('div', 'overview__bonus');
    const bonus = this._scoreEditor({
      label: t('webRemote.artists.bonus'),
      current: () => this._displayedBonus(),
      onStep: (next) => this._setBonus(next),
    });
    const bonusDefault = el('span', 'overview__bonus-default');
    bonusRow.append(el('span', 'overview__bonus-label', t('webRemote.artists.bonus')), bonus, bonusDefault);

    // Add a favourite by name, and filter the list.
    const tools = el('div', 'overview__tools');
    const addForm = el('form', 'overview__add');
    const addInput = el('input', 'overview__input');
    addInput.type = 'text';
    addInput.placeholder = t('webRemote.artists.addPlaceholder');
    addInput.setAttribute('aria-label', t('webRemote.artists.addPlaceholder'));
    const addBtn = el('button', 'overview__button', t('webRemote.artists.add'));
    addBtn.type = 'submit';
    addForm.append(addInput, addBtn);
    addForm.addEventListener('submit', (e) => {
      e.preventDefault();
      this._addFavourite(addInput.value);
      addInput.value = '';
    });
    const list = el('div', 'overview__list overview__list--artists');
    const filter = this._filterInput(t('overview.artists.filter'), this._artistFilter, (value) => {
      this._artistFilter = value;
      this._fillArtistList(list, this._artistRowsShown || []);
    });
    tools.append(addForm, filter);

    card.replaceChildren(head, hint, bonusRow, tools, list);
    this._artistUi = { card, count, bonus, bonusDefault, list };
    return this._artistUi;
  }

  _fillArtistList(list, rows) {
    const q = fold(this._artistFilter);
    const shown = q ? rows.filter((a) => fold(a.name).includes(q)) : rows;
    const byKey = new Map(shown.map((a) => [fold(a.name), a]));
    const existing = [...list.querySelectorAll('.overview__row[data-artist]')];
    if (existing.length && existing.length === byKey.size && existing.every((row) => byKey.has(row.dataset.artist))) {
      for (const row of existing) this._updateArtistRow(row, byKey.get(row.dataset.artist));
      return;
    }
    const scroll = list.scrollTop;
    list.replaceChildren(...shown.map((a) => {
      const row = el('div', 'overview__row');
      row.dataset.artist = fold(a.name);
      const star = el('button', 'overview__star');
      star.type = 'button';
      star.appendChild(icon(Star, { width: 16, height: 16 }));
      // Toggles what the star shows (a change still being applied included).
      star.addEventListener('click', () => this._setFavourite(row.dataset.name, star.getAttribute('aria-pressed') !== 'true'));
      row.append(star, el('span', 'overview__row-name'), el('span', 'overview__row-meta'));
      this._updateArtistRow(row, a);
      return row;
    }));
    if (!shown.length) list.appendChild(el('p', 'overview__empty', t('overview.noMatch')));
    list.scrollTop = scroll;
  }

  _updateArtistRow(row, a) {
    row.dataset.name = a.name;
    row.classList.toggle('overview__row--pending', !!a.pending);
    const star = row.querySelector('.overview__star');
    const label = t(a.favourite ? 'webRemote.artists.unfavourite' : 'webRemote.artists.favourite', { artist: a.name });
    star.classList.toggle('overview__star--on', a.favourite);
    star.title = label;
    star.setAttribute('aria-label', label);
    star.setAttribute('aria-pressed', a.favourite ? 'true' : 'false');
    row.querySelector('.overview__row-name').textContent = a.name;
    row.querySelector('.overview__row-meta').textContent = a.videos
      ? t('overview.videoCount', { count: a.videos })
      : t('webRemote.artists.notInLibrary');
  }

  _addFavourite(value) {
    const name = String(value || '').trim();
    if (!name) return;
    // Same artist spelled differently: star the one already listed.
    const known = (this._artistRowsShown || []).find((a) => fold(a.name) === fold(name));
    if (known?.favourite) {
      showToast(t('webRemote.artists.alreadyFavourite', { artist: known.name }), 'info');
      return;
    }
    this._setFavourite(known?.name || name, true);
  }

  _setFavourite(name, favourite) {
    const key = fold(name);
    this._artistUnsent.set(key, favourite);
    this._repaint(['artists']);
    this._run({ kind: 'favourite-artist', artist: name, favourite }, () => {
      if (this._artistUnsent.get(key) === favourite) this._artistUnsent.delete(key);
      this._repaint(['artists', 'tags']);
    }, t(favourite ? 'webRemote.artists.added' : 'webRemote.artists.removed', { artist: name }));
  }

  _setBonus(score) {
    this._bonusUnsent = score;
    this._repaint(['artists']);
    this._run({ kind: 'favourite-score', score }, () => {
      if (this._bonusUnsent === score) this._bonusUnsent = undefined;
      // The built-in weights line names the bonus too.
      this._repaint(['artists', 'tags']);
    });
  }

  // --- Tag scores ------------------------------------------------------------
  //
  // Built once per render like the artists card; rows keep their order while
  // the same tags are listed.

  _displayedTagScore(tag) {
    if (this._tagUnsent.has(tag)) return this._tagUnsent.get(tag);
    return effectiveTagScores(this._settings.get('library.tagScores'))[tag] ?? 0;
  }

  _paintTags(card) {
    const info = tagScoreInfo();
    const rows = tagSummary(this._settings, this._videoPaths(), info.groups).map((r) => (
      this._tagUnsent.has(r.tag) ? { ...r, score: this._tagUnsent.get(r.tag), pending: true } : r));
    this._tagRowsShown = rows;
    const ui = this._tagUi?.card === card ? this._tagUi : this._buildTagsCard(card);
    ui.count.textContent = t('overview.tags.count', { count: rows.length });
    const rules = RULE_ORDER.filter((id) => typeof info.rules[id] === 'number' && info.rules[id] !== 0)
      .map((id) => t(`webRemote.tagScores.rules.${id}`, { score: formatTagScore(id === 'favouriteArtist' ? favouriteScoreOf(this._settings) : info.rules[id]) }))
      .join(' · ');
    ui.rules.textContent = t('webRemote.tagScores.builtIn', { rules });
    ui.recalc.disabled = this._recalculating;
    ui.recalc.classList.toggle('overview__button--busy', this._recalculating);
    ui.recalcLabel.textContent = t(this._recalculating ? 'webRemote.tagScores.recalculating' : 'webRemote.tagScores.recalculate');
    this._fillTagList(ui.list, rows);
  }

  _buildTagsCard(card) {
    const count = el('span', 'overview__card-count');
    const head = this._cardTitle(t('overview.tags.title'), count);
    const hint = el('p', 'overview__hint', t('webRemote.tagScores.hint'));
    const rules = el('p', 'overview__hint overview__hint--rules');

    const tools = el('div', 'overview__tools');
    const list = el('div', 'overview__list overview__list--tags');
    const filter = this._filterInput(t('overview.tags.filter'), this._tagFilter, (value) => {
      this._tagFilter = value;
      this._fillTagList(list, this._tagRowsShown || []);
    });
    const recalc = el('button', 'overview__button');
    recalc.type = 'button';
    const recalcLabel = el('span');
    recalc.append(icon(RefreshCw, { width: 14, height: 14 }), recalcLabel);
    recalc.addEventListener('click', () => this._recalculate());
    tools.append(filter, recalc);

    card.replaceChildren(head, hint, rules, tools, list);
    this._tagUi = { card, count, rules, recalc, recalcLabel, list };
    return this._tagUi;
  }

  _fillTagList(list, rows) {
    const q = this._tagFilter.trim().toLowerCase();
    const shown = q ? rows.filter((r) => r.tag.includes(q) || (r.group && r.group.toLowerCase().includes(q))) : rows;
    const byTag = new Map(shown.map((r) => [r.tag, r]));
    const existing = [...list.querySelectorAll('.overview__row[data-tag]')];
    if (existing.length && existing.length === byTag.size && existing.every((row) => byTag.has(row.dataset.tag))) {
      for (const row of existing) this._updateTagRow(row, byTag.get(row.dataset.tag));
      return;
    }
    const scroll = list.scrollTop;
    list.replaceChildren(...shown.map((r) => {
      const row = el('div', 'overview__row');
      row.dataset.tag = r.tag;
      row.appendChild(el('span', 'overview__row-name', r.tag));
      if (r.group) {
        const group = el('span', 'overview__chip', r.group);
        group.title = t('webRemote.tagScores.group', { group: r.group });
        row.appendChild(group);
      }
      row.appendChild(el('span', 'overview__row-meta'));
      row.appendChild(this._scoreEditor({
        label: r.tag,
        current: () => this._displayedTagScore(r.tag),
        onStep: (next) => this._setTagScore(r.tag, next),
      }));
      this._updateTagRow(row, r);
      return row;
    }));
    if (!shown.length) list.appendChild(el('p', 'overview__empty', rows.length ? t('overview.noMatch') : t('overview.tags.none')));
    list.scrollTop = scroll;
  }

  _updateTagRow(row, r) {
    row.classList.toggle('overview__row--pending', !!r.pending);
    row.querySelector('.overview__row-meta').textContent = t('overview.videoCount', { count: r.videos });
    this._updateScoreEditor(row.querySelector('.overview__score'), { label: r.tag, score: r.score, pending: !!r.pending, changed: r.changed });
  }

  _setTagScore(tag, score) {
    this._tagUnsent.set(tag, score);
    this._repaint(['tags']);
    this._run({ kind: 'tag-score', tag, score }, () => {
      if (this._tagUnsent.get(tag) === score) this._tagUnsent.delete(tag);
      this._repaint(['tags']);
    });
  }

  _recalculate() {
    if (this._recalculating) return;
    this._recalculating = true;
    this._repaint(['tags']);
    this._run({ kind: 'recalculate-scores' }, () => {
      this._recalculating = false;
      this._repaint(['tags']);
    }, t('webRemote.tagScores.recalculated'));
  }

  // --- Shared ----------------------------------------------------------------

  /**
   * Apply `edit` through the app's library-edit applier; `done` runs either
   * way, `successMessage` shows as a toast once it's applied.
   */
  _run(edit, done, successMessage) {
    Promise.resolve()
      .then(() => this._applyEdit({ ...edit, source: 'desktop' }))
      .then(() => { if (successMessage) showToast(successMessage, 'success'); })
      .catch((err) => {
        console.warn('[Overview] edit not applied:', err);
        showToast(t('overview.editFailed', { error: err?.message || String(err) }), 'error');
      })
      .finally(done);
  }

  /**
   * − / score / + for a tag or the bonus; `_updateScoreEditor` shows the
   * score. A tap steps from `current()` (the score as shown, a change still
   * being applied included) and calls `onStep(next)` with the new score.
   */
  _scoreEditor({ label, current, onStep }) {
    const root = el('div', 'overview__score');
    const down = el('button', 'overview__score-btn');
    down.type = 'button';
    down.appendChild(icon(Minus, { width: 14, height: 14 }));
    down.title = t('webRemote.tagScores.decrease', { tag: label });
    down.setAttribute('aria-label', down.title);
    const value = el('span', 'overview__score-value');
    const up = el('button', 'overview__score-btn');
    up.type = 'button';
    up.appendChild(icon(Plus, { width: 14, height: 14 }));
    up.title = t('webRemote.tagScores.increase', { tag: label });
    up.setAttribute('aria-label', up.title);
    const step = (direction) => {
      const from = current();
      const next = stepTagScore(from, direction);
      if (next !== null && next !== from) onStep(next);
    };
    down.addEventListener('click', () => step(-1));
    up.addEventListener('click', () => step(1));
    root.append(down, value, up);
    return root;
  }

  _updateScoreEditor(root, { label, score, pending = false, changed = false }) {
    root.classList.toggle('overview__score--pending', pending);
    root.classList.toggle('overview__score--changed', changed);
    const value = root.querySelector('.overview__score-value');
    value.textContent = formatTagScore(score);
    value.classList.toggle('overview__score-value--up', score > 0);
    value.classList.toggle('overview__score-value--down', score < 0);
    value.setAttribute('aria-label', t('webRemote.tagScores.scoreLabel', { tag: label, score: formatTagScore(score) }));
  }

  _filterInput(placeholder, value, onInput) {
    const input = el('input', 'overview__input overview__filter');
    input.type = 'search';
    input.placeholder = placeholder;
    input.setAttribute('aria-label', placeholder);
    input.value = value;
    input.addEventListener('input', () => onInput(input.value));
    return input;
  }
}

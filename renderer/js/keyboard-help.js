// keyboard-help — "press ? to see all shortcuts" overlay for the player
// view (renderer/js/keyboard.js).
//
// The overlay itself is a `Modal.open` — same dialog primitives as the
// rest of the app. The caller passes group-of-rows data. Nielsen #7
// (flexibility for experts — surface every binding without making
// novices learn them) + #10 (help in context).
//
// Group shape:
//   [
//     { title: 'Playback', rows: [['Space / K', 'Play / Pause'], ...] },
//     { title: 'Volume',   rows: [['M', 'Mute / Unmute'], ['Up / Down', 'Volume ±5%']] },
//     ...
//   ]

import { Modal } from '../components/modal.js';
import { t } from './i18n.js';
import { dataService } from './data-service.js';

/**
 * Open the keyboard-help overlay with the given groups.
 * @param {string} title — Modal title (e.g. "Player keyboard shortcuts")
 * @param {Array<{title: string, rows: Array<[string, string]>}>} groups
 */
export function openKeyboardHelp(title, groups) {
  Modal.open({
    title,
    onRender(body, close) {
      const wrap = document.createElement('div');
      wrap.className = 'kbd-help';
      for (const g of groups) {
        const sec = document.createElement('section');
        sec.className = 'kbd-help__section';
        const h = document.createElement('h3');
        h.className = 'kbd-help__title';
        h.textContent = g.title;
        sec.appendChild(h);
        const tbl = document.createElement('dl');
        tbl.className = 'kbd-help__list';
        for (const [keys, desc] of g.rows) {
          const dt = document.createElement('dt');
          dt.className = 'kbd-help__keys';
          // Render each key part as <kbd> so screen readers + copy/paste
          // both render keyboard input semantically. Splits on " / "
          // (alternative bindings) but preserves "+" combos as one chip.
          for (const part of keys.split(/\s*\/\s*/)) {
            if (dt.children.length) dt.appendChild(document.createTextNode(' / '));
            const kbd = document.createElement('kbd');
            kbd.textContent = part;
            dt.appendChild(kbd);
          }
          const dd = document.createElement('dd');
          dd.className = 'kbd-help__desc';
          dd.textContent = desc;
          tbl.appendChild(dt);
          tbl.appendChild(dd);
        }
        sec.appendChild(tbl);
        wrap.appendChild(sec);
      }
      body.appendChild(wrap);

      const actions = document.createElement('div');
      actions.className = 'modal-actions';
      const ok = document.createElement('button');
      ok.className = 'modal-btn modal-btn--primary';
      ok.textContent = t('common.gotIt');
      ok.addEventListener('click', () => close());
      actions.appendChild(ok);
      body.appendChild(actions);
    },
  });
}

/**
 * Pre-built group set for the player view. Centralised so the
 * shortcut documentation stays in one place — anyone updating a
 * binding (in app.js, video-player.js) updates this list too.
 */
// Group sets are exported as builder functions so each `t()` call resolves
// against the active locale at modal-open time. Keyboard combos stay literal
// (they're the same in every language); only the row descriptions and group
// titles route through i18n.
export function getPlayerShortcutGroups() {
  return [
    {
      title: t('kbd.navigation'),
      rows: [
        ['Alt+1',         t('kbd.library')],
        ['Alt+2',         t('kbd.playlists')],
        ['Alt+3',         t('kbd.categories')],
        ['Alt+4',         t('kbd.overview')],
      ],
    },
    {
      title: t('kbd.playback'),
      rows: [
        ['Space / K',     t('kbd.playPause')],
        ['J / L',         t('kbd.seek10s')],
        ['Left / Right',  t('kbd.seek5s')],
        ['Shift+Left / Right', t('kbd.seekFrame')],
        ['Home / End',    t('kbd.jumpStartEnd')],
        ['G',             t('kbd.skipNextAction')],
        ['Shift+G',       t('kbd.skipPrevAction')],
        ['N',             t('kbd.nextVideo')],
        ['P',             t('kbd.prevVideo')],
      ],
    },
    {
      title: t('kbd.volume'),
      rows: [
        ['Up / Down',     t('kbd.volume5')],
        ['M',             t('kbd.muteToggle')],
      ],
    },
    {
      title: t('kbd.view'),
      rows: [
        ['F / F11',       t('kbd.fullscreen')],
        ['R',             t('kbd.cycleAspect')],
        ['Shift+R',       t('kbd.vrFlatten')],
        ['Shift+Arrows',  t('kbd.vrPan')],
        ['Ctrl+Scroll',   t('kbd.vrZoom')],
        ['Ctrl+0',        t('kbd.vrZoomReset')],
        ['<',             t('kbd.speedDown')],
        ['>',             t('kbd.speedUp')],
        ['I',             t('kbd.toggleInfo')],
        ['S',             t('kbd.screenshot')],
      ],
    },
    {
      title: t('kbd.abLoop'),
      rows: [
        ['A',             t('kbd.loopA')],
        ['B',             t('kbd.loopB')],
        ['Shift+L',       t('kbd.toggleVideoLoop')],
        ['Shift+Q',       t('kbd.toggleQueue')],
        ['Esc',           t('kbd.clearLoop')],
      ],
    },
    {
      title: t('kbd.devicesSync'),
      rows: [
        ['H',             t('kbd.devicesPanel')],
        // X was never listed here — a user with a configured finisher had no
        // way to discover the hotkey (lettuce, EroScripts #302).
        // Both keys can run as hold or press-to-toggle, so the row has to
        // say which one is armed — "(hold)" next to a key that toggles is
        // worse than no label at all.
        ['X',             dataService?.get?.('player.orgasmSwitchMode') === 'toggle'
                            ? t('kbd.orgasmSwitchToggle') : t('kbd.orgasmSwitch')],
        ['Z',             dataService?.get?.('player.edgeHoldMode') === 'toggle'
                            ? t('kbd.edgeHoldToggle') : t('kbd.edgeHold')],
      ],
    },
    {
      title: t('kbd.script'),
      rows: [
        ['V',             t('kbd.nextVariant')],
        ['Shift+V',       t('kbd.prevVariant')],
      ],
    },
    {
      title: t('kbd.chaptersBookmarks'),
      rows: [
        [',',             t('kbd.prevChapter')],
        ['.',             t('kbd.nextChapter')],
        ['Shift+B',       t('kbd.prevBookmark')],
        ['Ctrl+B',        t('kbd.nextBookmark')],
      ],
    },
    {
      title: t('kbd.libraryGroup'),
      rows: [
        ['O',             t('kbd.openFile')],
        ['Esc',           t('kbd.backToLibrary')],
      ],
    },
    {
      title: t('kbd.help'),
      rows: [
        ['?',             t('kbd.showHelp')],
      ],
    },
  ];
}

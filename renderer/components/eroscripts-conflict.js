// "This script already exists" prompt for EroScripts downloads.
// Resolves 'replace', 'keep', or null (cancel / dismissed).

import { Modal } from './modal.js';
import { t } from '../js/i18n.js';

export function askSaveConflict(fileName) {
  return Modal.open({
    title: t('eroscripts.conflictTitle'),
    onRender(body, close) {
      const msg = document.createElement('div');
      msg.className = 'modal-message';
      msg.textContent = t('eroscripts.conflictMessage', { name: fileName });
      body.appendChild(msg);

      const actions = document.createElement('div');
      actions.className = 'modal-actions';
      const button = (label, value, kind) => {
        const b = document.createElement('button');
        b.className = `modal-btn modal-btn--${kind}`;
        b.textContent = label;
        b.addEventListener('click', () => close(value));
        actions.appendChild(b);
      };
      button(t('common.cancel'), null, 'secondary');
      button(t('eroscripts.conflictKeepBoth'), 'keep', 'secondary');
      button(t('eroscripts.conflictReplace'), 'replace', 'danger');
      body.appendChild(actions);
    },
  });
}

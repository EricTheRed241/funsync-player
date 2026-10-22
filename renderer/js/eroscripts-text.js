// User-facing wording shared by the EroScripts panel and the Associate
// dialog's search, so both explain a post the same way.

import { t } from './i18n.js';
import { externalHostNames } from './eroscripts-groups.js';

/** Why a post offers nothing to download, in the user's terms. */
export function noFilesMessage({ paid = false, external = [] } = {}) {
  if (paid) return t('eroscripts.paidNoFiles');
  const hosts = externalHostNames(external);
  if (hosts.length > 0) return t('eroscripts.externalOnly', { hosts: hosts.join(', ') });
  return t('eroscripts.noAttachments');
}

/** "@user · post 12 · pixeldrain" for anything but the author's own forum attachment. */
export function attachmentSource(att) {
  const bits = [];
  if (att?.byAuthor === false && att.username) bits.push(`@${att.username}`);
  if (att?.postNumber > 1) bits.push(t('eroscripts.postNumber', { n: att.postNumber }));
  if (att?.host === 'pixeldrain') bits.push('pixeldrain');
  return bits.join(' · ');
}

// "No thank you" rule, renderer side: every video in the No thank you
// playlist is deleted from disk (electron/rejected-videos.js, through the
// `delete-rejected-video` IPC) and taken out of every playlist. Runs at
// startup and whenever a video lands in that playlist, from the desktop,
// the web remote or HereSphere (all go through the data service).

export const REJECT_PLAYLIST = 'No thank you';

/** True for the reject playlist's name, whatever its case or spacing. */
export function isRejectPlaylist(name) {
  return String(name || '').trim().toLocaleLowerCase() === REJECT_PLAYLIST.toLocaleLowerCase();
}

/**
 * Delete every video in the reject playlist. A video whose deletion fails
 * stays in the playlist, so the next sweep tries again.
 *
 * @param {object} o
 * @param {object} o.data DataService (getPlaylists, removeVideoFromPlaylist)
 * @param {(videoPath: string) => Promise<{success: boolean, error?: string}>} o.deleteVideo
 * @returns {Promise<{removed: string[], failed: {path: string, error: string}[]}>}
 */
export async function sweepRejected({ data, deleteVideo }) {
  const removed = [];
  const failed = [];
  const reject = data.getPlaylists().find((p) => isRejectPlaylist(p.name));
  for (const videoPath of [...(reject?.videoPaths || [])]) {
    const result = await deleteVideo(videoPath);
    if (!result?.success) {
      failed.push({ path: videoPath, error: result?.error || 'unknown error' });
      continue;
    }
    for (const pl of data.getPlaylists()) {
      if ((pl.videoPaths || []).includes(videoPath)) data.removeVideoFromPlaylist(pl.id, videoPath);
    }
    removed.push(videoPath);
  }
  return { removed, failed };
}

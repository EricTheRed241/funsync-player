//! Count each performer's scenes and decide which tags apply.

use crate::library::Video;
use crate::performers::Performer;
use crate::store::{Pornstar, PornstarFile};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct Tally {
    pub file: PornstarFile,
    /// FapTap performer names no performer answers to → their scene count.
    /// Reported as candidates, never applied.
    pub candidates: BTreeMap<String, usize>,
}

/// Every performer's distinct scene count over `videos`; a tag is applied
/// when it reaches `min_scenes`, and only applied tags go on videos.
/// `updated_at` is left empty for the caller.
pub fn tally(performers: &[Performer], videos: &[Video], min_scenes: usize) -> Tally {
    let matched: Vec<Vec<usize>> = videos
        .iter()
        .map(|v| {
            performers
                .iter()
                .enumerate()
                .filter(|(_, p)| p.phrases.iter().any(|ph| v.text.contains(ph)))
                .map(|(i, _)| i)
                .collect()
        })
        .collect();

    let mut scenes: Vec<HashSet<&str>> = vec![HashSet::new(); performers.len()];
    for (v, hits) in videos.iter().zip(&matched) {
        for &i in hits {
            scenes[i].insert(&v.scene);
        }
    }
    let applied: Vec<bool> = scenes.iter().map(|s| s.len() >= min_scenes).collect();

    let pornstars = performers
        .iter()
        .zip(&scenes)
        .zip(&applied)
        .map(|((p, s), &applied)| {
            (
                p.tag.clone(),
                Pornstar {
                    name: p.name.clone(),
                    scenes: s.len(),
                    applied,
                },
            )
        })
        .collect();

    let mut tagged = BTreeMap::new();
    for (v, hits) in videos.iter().zip(&matched) {
        let mut tags: Vec<String> = hits
            .iter()
            .filter(|&&i| applied[i])
            .map(|&i| performers[i].tag.clone())
            .collect();
        if tags.is_empty() {
            continue;
        }
        tags.sort();
        tags.dedup();
        tagged.insert(v.path.clone(), tags);
    }

    let mut candidate_scenes: BTreeMap<String, HashSet<&str>> = BTreeMap::new();
    for v in videos {
        for name in &v.faptap_performers {
            if !performers.iter().any(|p| p.is_named(name)) {
                candidate_scenes
                    .entry(name.clone())
                    .or_default()
                    .insert(&v.scene);
            }
        }
    }

    Tally {
        file: PornstarFile::new(min_scenes, pornstars, tagged),
        candidates: candidate_scenes
            .into_iter()
            .map(|(name, s)| (name, s.len()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performers;
    use crate::prefs::Listed;
    use crate::text::Text;

    fn listed(name: &str) -> Listed {
        Listed {
            name: name.into(),
            tag: None,
            aliases: Vec::new(),
            scenes: Vec::new(),
        }
    }

    fn video(path: &str, scene: &str, parts: &[&str], faptap: &[&str]) -> Video {
        Video {
            path: path.into(),
            scene: scene.into(),
            folder: scene.rsplit('/').next().unwrap().into(),
            text: Text::new(parts),
            faptap_performers: faptap.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn two_cuts_of_one_scene_count_once_and_one_scene_stays_pending() {
        let ps = performers::all(&[listed("Candy Love"), listed("Riley Reid")], &[]);
        let videos = [
            video("/S/a/one/cut1.mp4", "/S/a/one", &["Candy Love - One"], &[]),
            video("/S/a/one/cut2.mp4", "/S/a/one", &["Candy Love - One"], &[]),
            video("/S/b/two/v.mp4", "/S/b/two", &["Riley Reid - Two"], &[]),
        ];
        let t = tally(&ps, &videos, 2);
        assert_eq!(t.file.pornstars["candy-love"].scenes, 1);
        assert!(!t.file.pornstars["candy-love"].applied);
        assert!(t.file.videos.is_empty());

        // A second scene folder reaches the threshold: every video is tagged.
        let mut more = videos.to_vec();
        more.push(video(
            "/S/a/three/v.mp4",
            "/S/a/three",
            &["candy_love three"],
            &[],
        ));
        let t = tally(&ps, &more, 2);
        assert_eq!(t.file.pornstars["candy-love"].scenes, 2);
        assert!(t.file.pornstars["candy-love"].applied);
        assert!(!t.file.pornstars["riley-reid"].applied);
        assert_eq!(t.file.videos.len(), 3);
        assert_eq!(t.file.videos["/S/a/one/cut2.mp4"], ["candy-love"]);
        // With a threshold of 1 the single scene applies too.
        let t = tally(&ps, &more, 1);
        assert_eq!(t.file.videos["/S/b/two/v.mp4"], ["riley-reid"]);
    }

    #[test]
    fn aliases_scene_phrases_and_several_performers() {
        let mut vanna = listed("Vanna Bardot");
        vanna.scenes.push("VRBangers - A Gentlemans Wish".into());
        let mut piper = listed("Piper Presley");
        piper.aliases.push("whoispiperpresley".into());
        let ps = performers::all(
            &[vanna, piper, listed("Mia Malkova"), listed("Riley Reid")],
            &[],
        );
        let videos = [
            video(
                "/S/v/1/a.mp4",
                "/S/v/1",
                &["VRBangers - A Gentlemans Wish"],
                &[],
            ),
            video("/S/v/2/a.mp4", "/S/v/2", &["Vanna Bardot - Other"], &[]),
            video(
                "/S/p/1/a.mp4",
                "/S/p/1",
                &["x", "whoispiperpresley-pov-riding"],
                &[],
            ),
            video("/S/p/2/a.mp4", "/S/p/2", &["Piper Presley rides"], &[]),
            video(
                "/S/m/1/a.mp4",
                "/S/m/1",
                &["Mia Malkova, Riley Reid - One Last Bang"],
                &[],
            ),
            video("/S/m/2/a.mp4", "/S/m/2", &["Mia Malkova - Two"], &[]),
            video("/S/r/2/a.mp4", "/S/r/2", &["Riley Reid - Two"], &[]),
        ];
        let t = tally(&ps, &videos, 2);
        assert_eq!(t.file.pornstars["vanna-bardot"].scenes, 2);
        assert_eq!(t.file.pornstars["piper-presley"].scenes, 2);
        assert_eq!(t.file.videos["/S/v/1/a.mp4"], ["vanna-bardot"]);
        assert_eq!(t.file.videos["/S/p/1/a.mp4"], ["piper-presley"]);
        assert_eq!(t.file.videos["/S/m/1/a.mp4"], ["mia-malkova", "riley-reid"]);
    }

    #[test]
    fn unknown_faptap_performers_are_candidates_only() {
        let ps = performers::all(&[listed("Candy Love")], &[]);
        let videos = [
            video(
                "/S/a/1/a.mp4",
                "/S/a/1",
                &["Candy Love - x"],
                &["Candy Love", "Alice X"],
            ),
            video("/S/a/1/b.mp4", "/S/a/1", &["Candy Love - x"], &["Alice X"]),
            video("/S/a/2/a.mp4", "/S/a/2", &["Mercy"], &["Mercy"]),
        ];
        let t = tally(&ps, &videos, 1);
        assert_eq!(
            t.candidates,
            BTreeMap::from([("Alice X".to_string(), 1), ("Mercy".to_string(), 1)])
        );
        assert!(!t.file.pornstars.contains_key("mercy"));
        assert_eq!(t.file.videos["/S/a/1/a.mp4"], ["candy-love"]);
        assert!(!t.file.videos.contains_key("/S/a/2/a.mp4"));
    }
}

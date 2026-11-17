//! The sites this scout walks. VRBangers' group runs every site on the same
//! platform: a content API at `content.<site>/api/content/v1`, videos and
//! toy scripts on `mov.<site>` in a `<prefix>_<short name>/` folder, scene
//! pages at `<site>/video/<slug>/`. Another site of the group (BlowVR,
//! ARPorn) is one more row here.

/// One site of the platform.
#[derive(Debug, PartialEq, Eq)]
pub struct Site {
    /// The key in `--sites`, the scene's `site` field and `vrbScout.popularityPer`.
    pub key: &'static str,
    /// The studio name: in scene names (`… (VRBangers)`), as the scene's studio.
    pub name: &'static str,
    pub web: &'static str,
    pub content: &'static str,
    pub mov: &'static str,
    /// The video folder's prefix: `vrb_<short name>/`.
    pub folder_prefix: &'static str,
    /// The file names' prefix: `VRBANGERS_<short name>_8K_180x180_3dh.mp4`.
    pub file_prefix: &'static str,
    /// The user's account is a member here (vb_hack.txt is its login): a
    /// refused login stops `--fetch` (exit 3). On the other sites it only
    /// skips their scenes ("needs a membership").
    pub member: bool,
}

pub const SITES: &[Site] = &[
    Site {
        key: "vrbangers",
        name: "VRBangers",
        web: "https://vrbangers.com",
        content: "https://content.vrbangers.com",
        mov: "https://mov.vrbangers.com",
        folder_prefix: "vrb",
        file_prefix: "VRBANGERS",
        member: true,
    },
    Site {
        key: "vrconk",
        name: "VRConk",
        web: "https://vrconk.com",
        content: "https://content.vrconk.com",
        mov: "https://mov.vrconk.com",
        folder_prefix: "vrconk",
        file_prefix: "VRCONK",
        member: false,
    },
];

/// Videos per listing page (the API's own maximum is higher; 100 keeps
/// answers small and the walk at ~10 pages for VRBangers).
pub const PAGE_SIZE: u32 = 100;

pub fn by_key(key: &str) -> Option<&'static Site> {
    SITES.iter().find(|s| s.key == key)
}

impl Site {
    pub fn api(&self) -> String {
        format!("{}/api/content/v1", self.content)
    }

    /// A listing page, newest first (`sort` and `limit` are required).
    pub fn list_url(&self, page: u32) -> String {
        format!(
            "{}/videos?page={page}&limit={PAGE_SIZE}&sort=latest",
            self.api()
        )
    }

    pub fn detail_url(&self, slug: &str) -> String {
        format!("{}/videos/{slug}", self.api())
    }

    /// The site's lookup tables (fps, degrees, formats by id).
    pub fn digest_url(&self) -> String {
        format!("{}/digest", self.api())
    }

    pub fn scene_url(&self, slug: &str) -> String {
        format!("{}/video/{slug}/", self.web)
    }

    pub fn login_url(&self) -> String {
        format!("{}/login/", self.web)
    }

    /// The public toy script: `https://mov.<site>/<prefix>_<short>/MISC/<PREFIX>_<short>_toy.funscript`.
    pub fn script_url(&self, short: &str) -> String {
        format!(
            "{}/{}_{short}/MISC/{}_{short}_toy.funscript",
            self.mov, self.folder_prefix, self.file_prefix
        )
    }

    /// An image permalink as an https URL: relative ones are under
    /// `content.<site>`; plain http is dropped (the deck is served over https
    /// to the headset's browser).
    pub fn asset(&self, permalink: &str) -> Option<String> {
        let p = permalink.trim();
        if p.is_empty() {
            None
        } else if p.starts_with("https://") {
            Some(p.to_string())
        } else if p.starts_with("//") {
            Some(format!("https:{p}"))
        } else if p.contains("://") {
            None
        } else {
            Some(format!(
                "{}{}{p}",
                self.content,
                if p.starts_with('/') { "" } else { "/" }
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_urls() {
        let vrb = by_key("vrbangers").unwrap();
        assert_eq!(
            vrb.list_url(3),
            "https://content.vrbangers.com/api/content/v1/videos?page=3&limit=100&sort=latest"
        );
        assert_eq!(
            vrb.detail_url("wet-and-wild"),
            "https://content.vrbangers.com/api/content/v1/videos/wet-and-wild"
        );
        assert_eq!(
            vrb.scene_url("wet-and-wild"),
            "https://vrbangers.com/video/wet-and-wild/"
        );
        assert_eq!(
            vrb.script_url("sin_sisters_a_porn_parody"),
            "https://mov.vrbangers.com/vrb_sin_sisters_a_porn_parody/MISC/VRBANGERS_sin_sisters_a_porn_parody_toy.funscript"
        );
        let conk = by_key("vrconk").unwrap();
        assert_eq!(
            conk.script_url("vrcosplayx_the_substance_a_porn_parody"),
            "https://mov.vrconk.com/vrconk_vrcosplayx_the_substance_a_porn_parody/MISC/VRCONK_vrcosplayx_the_substance_a_porn_parody_toy.funscript"
        );
        assert_eq!(conk.login_url(), "https://vrconk.com/login/");
        assert!(by_key("blowvr").is_none());
    }

    #[test]
    fn makes_image_links_https() {
        let s = by_key("vrbangers").unwrap();
        assert_eq!(
            s.asset("/uploads/2026/10/a.jpg").as_deref(),
            Some("https://content.vrbangers.com/uploads/2026/10/a.jpg")
        );
        assert_eq!(
            s.asset("uploads/a.jpg").as_deref(),
            Some("https://content.vrbangers.com/uploads/a.jpg")
        );
        assert_eq!(
            s.asset("https://cdn.x/a.jpg").as_deref(),
            Some("https://cdn.x/a.jpg")
        );
        assert_eq!(
            s.asset("//cdn.x/a.jpg").as_deref(),
            Some("https://cdn.x/a.jpg")
        );
        assert_eq!(s.asset("http://cdn.x/a.jpg"), None);
        assert_eq!(s.asset(" "), None);
    }
}

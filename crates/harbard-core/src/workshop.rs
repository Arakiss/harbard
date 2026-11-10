//! Agent names: Nora, Owen, Iris, Hazel…
//!
//! Every agent carries a realistic English given name from a curated list,
//! each with a gender so the Spanish role agrees: "Nora · reviewer", "Owen ·
//! reviewer". The second line is `<role> · <team>`, "builder ·
//! protocol". The lead stays "Claude · lead" and never takes one.
//!
//! Assignment ([`Registry::assign`]):
//! - stable per agent id, and persisted (`{home}/names.json`);
//! - unique among live agents;
//! - an agent keeps its name across rounds of the same role and team: a
//!   new agent of a team and role whose last holder is gone takes its name;
//! - a session may pick one from the list with a one-line reason
//!   (`harbard name Nora --reason "…"`), shown in Profile.
//!
//! Learning: a rename, a 👍 and a 👎 are feedback events in the same file.
//! A 👎'd name is never given again. The generator prefers the styles the operator
//! kept or renamed towards: a small score over a name's features (length,
//! ending, gender), learnt from those events.

use crate::coded::{Params, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Gender {
    F,
    M,
}

/// Given names read as a woman's: the role agrees in the feminine.
const WOMEN: &[&str] = &[
    "Abigail",
    "Ada",
    "Adele",
    "Agnes",
    "Alice",
    "Alison",
    "Amanda",
    "Amber",
    "Amelia",
    "Amy",
    "Andrea",
    "Angela",
    "Anita",
    "Anna",
    "Annie",
    "Audrey",
    "Ava",
    "Barbara",
    "Beatrice",
    "Bella",
    "Bethany",
    "Bianca",
    "Brenda",
    "Bridget",
    "Brooke",
    "Caitlin",
    "Camille",
    "Carla",
    "Carmen",
    "Carol",
    "Caroline",
    "Cassandra",
    "Catherine",
    "Cecilia",
    "Celia",
    "Charlotte",
    "Chloe",
    "Christine",
    "Claire",
    "Clara",
    "Claudia",
    "Colleen",
    "Cynthia",
    "Daisy",
    "Danielle",
    "Deborah",
    "Diana",
    "Diane",
    "Donna",
    "Dorothy",
    "Edith",
    "Eileen",
    "Elaine",
    "Eleanor",
    "Elena",
    "Elise",
    "Eliza",
    "Elizabeth",
    "Ella",
    "Ellen",
    "Eloise",
    "Emily",
    "Emma",
    "Erica",
    "Erin",
    "Esther",
    "Eva",
    "Evelyn",
    "Fiona",
    "Florence",
    "Frances",
    "Gemma",
    "Georgia",
    "Gillian",
    "Gloria",
    "Hannah",
    "Harriet",
    "Hazel",
    "Heather",
    "Heidi",
    "Helen",
    "Holly",
    "Imogen",
    "Ingrid",
    "Irene",
    "Iris",
    "Isabel",
    "Ivy",
    "Jacqueline",
    "Jane",
    "Janet",
    "Jasmine",
    "Jean",
    "Jenna",
    "Jennifer",
    "Jessica",
    "Joan",
    "Joanna",
    "Jocelyn",
    "Josephine",
    "Joyce",
    "Judith",
    "Julia",
    "Juliet",
    "Kate",
    "Katherine",
    "Kathleen",
    "Kayla",
    "Kelly",
    "Kimberly",
    "Kirsten",
    "Laura",
    "Lauren",
    "Leah",
    "Leslie",
    "Lillian",
    "Linda",
    "Lisa",
    "Louise",
    "Lucy",
    "Lydia",
    "Lynn",
    "Mabel",
    "Madeline",
    "Maggie",
    "Margaret",
    "Maria",
    "Marian",
    "Marilyn",
    "Martha",
    "Mary",
    "Maureen",
    "Megan",
    "Melanie",
    "Melissa",
    "Meredith",
    "Michelle",
    "Miranda",
    "Molly",
    "Monica",
    "Nadia",
    "Naomi",
    "Natalie",
    "Nicole",
    "Nina",
    "Nora",
    "Olivia",
    "Pamela",
    "Patricia",
    "Paula",
    "Pauline",
    "Penelope",
    "Philippa",
    "Phoebe",
    "Polly",
    "Priscilla",
    "Rachel",
    "Rebecca",
    "Regina",
    "Renee",
    "Rita",
    "Robyn",
    "Rosalind",
    "Rosemary",
    "Ruby",
    "Ruth",
    "Sabrina",
    "Samantha",
    "Sandra",
    "Sara",
    "Sarah",
    "Scarlett",
    "Selena",
    "Shannon",
    "Sharon",
    "Sheila",
    "Shirley",
    "Sophia",
    "Sophie",
    "Stella",
    "Stephanie",
    "Susan",
    "Sylvia",
    "Tabitha",
    "Tamara",
    "Teresa",
    "Tessa",
    "Thea",
    "Tiffany",
    "Valerie",
    "Vanessa",
    "Vera",
    "Veronica",
    "Victoria",
    "Violet",
    "Virginia",
    "Vivian",
    "Wendy",
    "Whitney",
    "Yvonne",
    "Zoe",
];

/// Given names read as a man's.
const MEN: &[&str] = &[
    "Aaron",
    "Adam",
    "Adrian",
    "Aidan",
    "Alan",
    "Albert",
    "Alec",
    "Alexander",
    "Alfred",
    "Andrew",
    "Angus",
    "Anthony",
    "Archie",
    "Arnold",
    "Arthur",
    "Austin",
    "Barry",
    "Ben",
    "Benjamin",
    "Bernard",
    "Blake",
    "Bradley",
    "Brandon",
    "Brendan",
    "Brett",
    "Brian",
    "Bruce",
    "Bryan",
    "Caleb",
    "Calvin",
    "Cameron",
    "Carl",
    "Carter",
    "Casey",
    "Charles",
    "Christian",
    "Christopher",
    "Clark",
    "Clifford",
    "Clive",
    "Colin",
    "Connor",
    "Conrad",
    "Craig",
    "Curtis",
    "Cyril",
    "Dale",
    "Damian",
    "Daniel",
    "Darren",
    "David",
    "Dean",
    "Declan",
    "Dennis",
    "Derek",
    "Desmond",
    "Dominic",
    "Donald",
    "Douglas",
    "Duncan",
    "Dustin",
    "Dylan",
    "Edgar",
    "Edmund",
    "Edward",
    "Edwin",
    "Elliot",
    "Emmett",
    "Eric",
    "Ernest",
    "Ethan",
    "Eugene",
    "Evan",
    "Felix",
    "Fergus",
    "Finn",
    "Francis",
    "Fraser",
    "Frederick",
    "Gabriel",
    "Gareth",
    "Gary",
    "Gavin",
    "George",
    "Gerald",
    "Gilbert",
    "Glenn",
    "Gordon",
    "Graham",
    "Gregory",
    "Harold",
    "Harrison",
    "Harry",
    "Harvey",
    "Hector",
    "Henry",
    "Herbert",
    "Howard",
    "Hugh",
    "Hugo",
    "Ian",
    "Isaac",
    "Ivan",
    "Jack",
    "Jacob",
    "Jake",
    "James",
    "Jared",
    "Jason",
    "Jasper",
    "Jeffrey",
    "Jeremy",
    "Jerome",
    "Jesse",
    "Joel",
    "John",
    "Jonah",
    "Jonathan",
    "Jordan",
    "Joseph",
    "Joshua",
    "Julian",
    "Justin",
    "Keith",
    "Kenneth",
    "Kevin",
    "Kieran",
    "Kyle",
    "Lance",
    "Lawrence",
    "Leo",
    "Leonard",
    "Lewis",
    "Liam",
    "Lionel",
    "Logan",
    "Louis",
    "Lucas",
    "Luke",
    "Malcolm",
    "Marcus",
    "Martin",
    "Marvin",
    "Matthew",
    "Maurice",
    "Maxwell",
    "Michael",
    "Miles",
    "Mitchell",
    "Morgan",
    "Murray",
    "Nathan",
    "Neil",
    "Nelson",
    "Nicholas",
    "Nigel",
    "Noah",
    "Norman",
    "Oliver",
    "Oscar",
    "Owen",
    "Patrick",
    "Paul",
    "Percy",
    "Peter",
    "Philip",
    "Preston",
    "Quentin",
    "Ralph",
    "Randall",
    "Raymond",
    "Reginald",
    "Rex",
    "Richard",
    "Robert",
    "Rodney",
    "Roger",
    "Roland",
    "Ronald",
    "Rory",
    "Ross",
    "Roy",
    "Rupert",
    "Russell",
    "Ryan",
    "Samuel",
    "Scott",
    "Sean",
    "Sebastian",
    "Seth",
    "Shane",
    "Simon",
    "Spencer",
    "Stanley",
    "Stephen",
    "Stuart",
    "Terence",
    "Theodore",
    "Thomas",
    "Timothy",
    "Tobias",
    "Todd",
    "Tony",
    "Travis",
    "Trevor",
    "Tristan",
    "Troy",
    "Tyler",
    "Victor",
    "Vincent",
    "Walter",
    "Warren",
    "Wayne",
    "Wesley",
    "Wilfred",
    "William",
    "Xavier",
    "Zachary",
];

/// What tells two holders of one given name apart: "Nora Walsh".
const SURNAMES: &[&str] = &[
    "Abbott",
    "Adams",
    "Ashworth",
    "Bailey",
    "Barnes",
    "Bennett",
    "Blackwood",
    "Bradford",
    "Brooks",
    "Calloway",
    "Campbell",
    "Carver",
    "Chapman",
    "Clarke",
    "Coleman",
    "Collins",
    "Cooper",
    "Crawford",
    "Davies",
    "Dawson",
    "Donovan",
    "Ellis",
    "Emerson",
    "Fletcher",
    "Foster",
    "Gallagher",
    "Gibson",
    "Goodwin",
    "Hamilton",
    "Harper",
    "Hartley",
    "Hayes",
    "Holloway",
    "Hughes",
    "Jennings",
    "Keller",
    "Kennedy",
    "Lambert",
    "Lawson",
    "Lockwood",
    "Marshall",
    "Mercer",
    "Mitchell",
    "Montgomery",
    "Murphy",
    "Nash",
    "Norton",
    "Palmer",
    "Parker",
    "Porter",
    "Quinn",
    "Reed",
    "Reynolds",
    "Rowland",
    "Sawyer",
    "Shaw",
    "Sinclair",
    "Sullivan",
    "Thornton",
    "Turner",
    "Vaughn",
    "Wallace",
    "Walsh",
    "Whitfield",
    "Winslow",
];

/// How many given names there are.
pub fn name_count() -> usize {
    WOMEN.len() + MEN.len()
}

/// The `i`-th given name, in a fixed order, with its gender.
pub fn nth_name(i: usize) -> (&'static str, Gender) {
    let i = i % name_count();
    match WOMEN.get(i) {
        Some(n) => (n, Gender::F),
        None => (MEN[i - WOMEN.len()], Gender::M),
    }
}

/// Every given name, with its gender.
fn names() -> impl Iterator<Item = (&'static str, Gender)> {
    (0..name_count()).map(nth_name)
}

/// The list entry of a name, matched without case.
pub fn listed(name: &str) -> Option<(&'static str, Gender)> {
    // By ASCII-folded name, the first entry of each fold: the same answer
    // as the first entry `eq_ignore_ascii_case` matches, without the scan.
    static BY_FOLD: std::sync::LazyLock<HashMap<String, usize>> = std::sync::LazyLock::new(|| {
        let mut by = HashMap::new();
        for (i, (n, _)) in names().enumerate() {
            by.entry(n.to_ascii_lowercase()).or_insert(i);
        }
        by
    });
    BY_FOLD
        .get(&name.trim().to_ascii_lowercase())
        .map(|&i| nth_name(i))
}

/// A listed name in a pick's order: highest score first, then its hash.
type Ranked<'a> = (std::cmp::Reverse<(i32, u64)>, &'a str, Gender);

/// Per listed name, in list order: lower-cased, its features and its hash,
/// worked out once for every pick.
struct ListedKey {
    lower: String,
    features: Vec<String>,
    hash: u64,
}

fn listed_keys() -> &'static [ListedKey] {
    static K: std::sync::LazyLock<Vec<ListedKey>> = std::sync::LazyLock::new(|| {
        names()
            .map(|(n, _)| ListedKey {
                lower: n.to_lowercase(),
                features: features(n, None),
                hash: fnv(n),
            })
            .collect()
    });
    &K
}

/// What the console shows: the name, and `<role> · <team>` under it.
pub fn role_line(role: Option<&str>, team: Option<&str>) -> Option<String> {
    let role = role.map(crate::naming::role_text);
    match (role, team) {
        (Some(r), Some(t)) => Some(format!("{r} · {t}")),
        (Some(r), None) => Some(r),
        (None, Some(t)) => Some(t.to_string()),
        (None, None) => None,
    }
}

/// The console's code and params for a role line harbard generated: a known
/// role and its team. `None` for free text someone wrote, shown as it is. The
/// gender travels so a language that agrees the role can.
pub fn role_line_coded(
    role: Option<&str>,
    team: Option<&str>,
    gender: Gender,
) -> Option<(&'static str, Params)> {
    let (canonical, number) = crate::naming::known_role(role?)?;
    let mut values = params([
        ("role", canonical.into()),
        (
            "gender",
            match gender {
                Gender::F => "f",
                Gender::M => "m",
            }
            .into(),
        ),
    ]);
    if let Some(n) = number {
        values.insert("number".into(), n.into());
    }
    if let Some(t) = team {
        values.insert("team".into(), t.into());
    }
    let code = if team.is_some() {
        "role_line"
    } else {
        "role_line.solo"
    };
    Some((code, values))
}

/// One agent's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub gender: Gender,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    /// `auto` (the generator), `picked` (`harbard name`), `renamed` (the operator).
    pub source: String,
    /// A session's one-line reason for its pick.
    #[serde(default)]
    pub reason: Option<String>,
    pub assigned_ms: u64,
    /// `team` was said outright (`harbard name --team`), not derived: it
    /// wins over the name's suffix and the repository written in.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub team_explicit: bool,
}

/// A feedback event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vote {
    pub ts_ms: u64,
    pub agent: String,
    pub name: String,
    /// `up`, `down` or `rename` (from `name` to `to`).
    pub vote: String,
    #[serde(default)]
    pub to: Option<String>,
}

/// What the assignment needs to know of an agent.
#[derive(Debug, Clone, Default)]
pub struct Wanted {
    pub key: String,
    pub team: Option<String>,
    pub role: Option<String>,
    pub live: bool,
    pub first_ms: u64,
    /// The name it was launched with, when that is a listed name
    /// (a lead's `Agent` call named it "Laura").
    pub own: Option<String>,
}

/// `{home}/names.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub agents: BTreeMap<String, Entry>,
    #[serde(default)]
    pub votes: Vec<Vote>,
}

/// Feedback kept, newest last.
const VOTES_MAX: usize = 2_000;

fn features(name: &str, gender: Option<Gender>) -> Vec<String> {
    let n = name.chars().count();
    let len = if n <= 5 {
        "corto"
    } else if n <= 7 {
        "medio"
    } else {
        "largo"
    };
    let last = name
        .chars()
        .last()
        .map(|c| c.to_lowercase().collect::<String>())
        .unwrap_or_default();
    let ending = match last.as_str() {
        "a" => "a",
        "o" => "o",
        "e" => "e",
        _ => "otra",
    };
    let mut out = vec![format!("len:{len}"), format!("end:{ending}")];
    if let Some((_, g)) = listed(name) {
        out.push(format!("gen:{g:?}"));
    } else if let Some(g) = gender {
        out.push(format!("gen:{g:?}"));
    }
    out
}

/// A name the operator typed, cleaned: letters, spaces and hyphens, at most
/// 24 characters, first letter upper case. `None` when nothing is left.
pub fn clean_name(raw: &str) -> Option<String> {
    let kept: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_alphabetic() || *c == ' ' || *c == '-')
        .take(24)
        .collect();
    let kept = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = kept.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

impl Registry {
    pub fn path(root: &Path) -> PathBuf {
        root.join("names.json")
    }

    pub fn load(root: &Path) -> Self {
        static C: crate::stamped::Shared<Registry> = crate::stamped::Shared::new(Default::default);
        crate::stamped::json(&C, &Self::path(root))
    }

    /// Write by temp file and rename, so a reader never sees half a file.
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::create_dir_all(root)?;
        let tmp = root.join(format!(".names.{}.tmp", ulid::Ulid::generate()));
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, Self::path(root))
    }

    /// Read-modify-write under an exclusive lock beside the file.
    pub fn update<T>(root: &Path, f: impl FnOnce(&mut Registry) -> T) -> std::io::Result<T> {
        use fs2::FileExt;
        std::fs::create_dir_all(root)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(".names.lock"))?;
        lock.lock_exclusive()?;
        let mut reg = Self::load(root);
        let before = reg.clone();
        let out = f(&mut reg);
        if reg != before {
            reg.save(root)?;
        }
        let _ = FileExt::unlock(&lock);
        Ok(out)
    }

    /// Names a 👎 took out: never given again.
    pub fn rejected(&self) -> BTreeSet<String> {
        self.votes
            .iter()
            .filter(|v| v.vote == "down")
            .map(|v| v.name.to_lowercase())
            .collect()
    }

    /// How much the operator seems to like a name's style, from their feedback: a
    /// 👍 counts +1 for each feature of the name, a 👎 −2, a rename −1 for
    /// the name left and +2 for the name chosen.
    pub fn score(&self, name: &str) -> i32 {
        score_in(&self.weights(), name)
    }

    /// The weight of each name feature, from the feedback (see [`Registry::score`]).
    fn weights(&self) -> BTreeMap<String, i32> {
        let mut weight: BTreeMap<String, i32> = BTreeMap::new();
        let mut add = |n: &str, w: i32| {
            for f in features(n, None) {
                *weight.entry(f).or_default() += w;
            }
        };
        for v in &self.votes {
            match v.vote.as_str() {
                "up" => add(&v.name, 1),
                "down" => add(&v.name, -2),
                "rename" => {
                    add(&v.name, -1);
                    if let Some(to) = &v.to {
                        add(to, 2);
                    }
                }
                _ => {}
            }
        }
        weight
    }

    /// The names live agents hold, lower-cased.
    fn held(&self, live: &BTreeSet<&str>) -> BTreeSet<String> {
        self.agents
            .iter()
            .filter(|(k, _)| live.contains(k.as_str()))
            .map(|(_, e)| e.name.to_lowercase())
            .collect()
    }

    /// Would [`Registry::settle`] change anything? A dry run on a copy: the
    /// read takes the file lock only when it must write.
    pub fn needs_settle(&self, wanted: &[Wanted], taken: &BTreeSet<String>, now: u64) -> bool {
        self.clone().settle(wanted, taken, now)
    }

    /// The read's one write: settle who holds which name among the shown
    /// agents, then name the new ones. `taken` are the callsigns sessions
    /// hold, lower-cased: a new name avoids them, a name already given stays.
    /// Returns whether anything changed (the caller saves).
    pub fn settle(&mut self, wanted: &[Wanted], taken: &BTreeSet<String>, now: u64) -> bool {
        let retired = self.retire_old_names();
        let settled = self.reconcile(wanted, taken, now);
        self.assign(wanted, taken, now) || settled || retired
    }

    /// A name the generator gave from a list that no longer exists goes, and
    /// `assign` gives the agent a current one. A pick or a rename is somebody's
    /// choice and stays.
    fn retire_old_names(&mut self) -> bool {
        let before = self.agents.len();
        self.agents.retain(|_, e| {
            !matches!(e.source.as_str(), "auto" | "launched") || listed(base_of(&e.name)).is_some()
        });
        self.agents.len() != before
    }

    /// Two shown agents never read the same name, and a teammate reads the
    /// name it was launched with (AC4, on a names.json written by older
    /// rules). Each shown agent claims a name: its launched name when it
    /// has one and its entry is `auto` or `launched` (which repairs
    /// a surname it no longer needs); else the name it holds. Of the claims on one name, the
    /// strongest keeps it: the operator's rename, a session's pick, a launched
    /// name (the live teammate before a finished one), an auto one; the
    /// oldest among equals. An auto agent that loses
    /// takes a free name of the list, a suffix only once the list runs out;
    /// any other takes its claim with a surname ("Nora Walsh"). Keys never change,
    /// and an agent not shown is not touched.
    fn reconcile(&mut self, wanted: &[Wanted], taken: &BTreeSet<String>, now: u64) -> bool {
        struct Claim<'w> {
            w: &'w Wanted,
            name: String,
            rank: u8,
            launched: bool,
        }
        let rank = |source: &str| match source {
            "renamed" => 0,
            "picked" => 1,
            "launched" => 2,
            _ => 3,
        };
        let mut claims: Vec<Claim<'_>> = Vec::new();
        for w in wanted {
            let entry = self.agents.get(&w.key);
            let own = w.own.as_deref().and_then(listed).map(|(n, _)| n);
            let open = entry.is_none_or(|e| matches!(e.source.as_str(), "auto" | "launched"));
            // A session goes by this name: `assign` gives a new agent a
            // surname, and one that already has its name keeps it.
            let own = own.filter(|n| !taken.contains(&n.to_lowercase()));
            match (own, entry) {
                (Some(n), _) if open => claims.push(Claim {
                    w,
                    name: n.to_string(),
                    rank: 2,
                    launched: true,
                }),
                (_, Some(e)) => claims.push(Claim {
                    w,
                    name: e.name.clone(),
                    rank: rank(&e.source),
                    launched: false,
                }),
                // No entry and no launched name: `assign` names it.
                _ => {}
            }
        }
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, c) in claims.iter().enumerate() {
            by_name.entry(c.name.to_lowercase()).or_default().push(i);
        }
        let mut winners: Vec<usize> = Vec::new();
        let mut losers: Vec<usize> = Vec::new();
        for mut group in by_name.into_values() {
            group.sort_by(|&a, &b| {
                let (x, y) = (&claims[a], &claims[b]);
                // Two teammates launched with one name: the one at work
                // now is the one the operator talks to.
                // Then whoever holds it already, so the name does not
                // swap back once both have finished.
                let busy = |c: &Claim<'_>| !(c.launched && c.w.live);
                let holds = |c: &Claim<'_>| {
                    !self
                        .agents
                        .get(&c.w.key)
                        .is_some_and(|e| e.name.eq_ignore_ascii_case(&c.name))
                };
                x.rank
                    .cmp(&y.rank)
                    .then(busy(x).cmp(&busy(y)))
                    .then(holds(x).cmp(&holds(y)))
                    .then(x.w.first_ms.cmp(&y.w.first_ms))
                    .then(x.w.key.cmp(&y.w.key))
            });
            winners.push(group[0]);
            losers.extend_from_slice(&group[1..]);
        }
        let mut held: BTreeSet<String> = winners
            .iter()
            .map(|&i| claims[i].name.to_lowercase())
            .collect();
        held.extend(taken.iter().cloned());
        let rejected = self.rejected();
        losers.sort_by(|&a, &b| {
            let (x, y) = (&claims[a], &claims[b]);
            x.w.first_ms.cmp(&y.w.first_ms).then(x.w.key.cmp(&y.w.key))
        });
        let mut out: Vec<(usize, String)> = winners
            .iter()
            .map(|&i| (i, claims[i].name.clone()))
            .collect();
        for i in losers {
            let c = &claims[i];
            let next = if !c.launched && c.rank == 3 {
                self.fresh(&c.w.key, &held, &rejected).map(|(n, _)| n)
            } else {
                let base = base_of(&c.name);
                let base = listed(base).map_or(base, |(n, _)| n).to_string();
                (2..=SUFFIX_MAX)
                    .map(|r| suffixed(&base, r))
                    .find(|n| !held.contains(&n.to_lowercase()))
            };
            let Some(name) = next else { continue };
            held.insert(name.to_lowercase());
            out.push((i, name));
        }
        let mut changed = false;
        for (i, name) in out {
            let c = &claims[i];
            let source = if c.launched { Some("launched") } else { None };
            match self.agents.get_mut(&c.w.key) {
                Some(e) => {
                    if e.name != name {
                        if let Some((_, g)) = listed(base_of(&name)) {
                            e.gender = g;
                        }
                        e.name = name;
                        changed = true;
                    }
                    if let Some(s) = source
                        && e.source != s
                    {
                        e.source = s.into();
                        changed = true;
                    }
                }
                None => {
                    let gender = listed(base_of(&name)).map_or(Gender::F, |(_, g)| g);
                    self.agents.insert(
                        c.w.key.clone(),
                        Entry {
                            name,
                            gender,
                            team: c.w.team.clone(),
                            role: c.w.role.clone(),
                            source: source.unwrap_or("auto").into(),
                            reason: None,
                            assigned_ms: now,
                            team_explicit: false,
                        },
                    );
                    changed = true;
                }
            }
        }
        changed
    }

    /// Give every wanted agent with no name one. Returns whether anything
    /// changed (the caller saves).
    ///
    /// Unique among the agents the console shows, live or finished: two
    /// rows never read the same (harbard-final-review AC4). In order:
    /// - the workshop name it was launched with, when no shown agent holds it;
    /// - the name of a predecessor of the same team and role that has left
    ///   the list (a new round), when both have a role: a role of None is
    ///   no role, and matches nothing;
    /// - the best free name of the list;
    /// - past the list, the same names with a surname ("Nora Walsh").
    pub fn assign(&mut self, wanted: &[Wanted], taken: &BTreeSet<String>, now: u64) -> bool {
        let shown: BTreeSet<&str> = wanted.iter().map(|w| w.key.as_str()).collect();
        let mut order: Vec<&Wanted> = wanted
            .iter()
            .filter(|w| !self.agents.contains_key(&w.key))
            .collect();
        // A launched name is claimed before the list hands it to another.
        order.sort_by(|a, b| {
            a.own
                .is_none()
                .cmp(&b.own.is_none())
                .then(a.first_ms.cmp(&b.first_ms))
                .then(a.key.cmp(&b.key))
        });
        let mut changed = false;
        for w in order {
            let mut held = self.held(&shown);
            held.extend(taken.iter().cloned());
            let rejected = self.rejected();
            let free = |n: &str| {
                let l = n.to_lowercase();
                !held.contains(&l) && !rejected.contains(&l)
            };
            // Its own name; when another shown agent holds it, the same
            // with a surname ("Nora Walsh"): the lead still calls it Nora.
            let launched = w.own.as_deref().and_then(listed).and_then(|(n, g)| {
                (1..=SUFFIX_MAX)
                    .map(|r| suffixed(n, r))
                    .find(|c| free(c))
                    .map(|c| (c, g, "launched"))
            });
            // The same team and role as an agent the list no longer shows.
            let kept = || {
                if w.team.is_none() || w.role.is_none() {
                    return None;
                }
                self.agents
                    .iter()
                    .filter(|(k, e)| {
                        !shown.contains(k.as_str())
                            && e.team == w.team
                            && e.role == w.role
                            && free(&e.name)
                    })
                    .max_by_key(|(_, e)| e.assigned_ms)
                    .map(|(_, e)| (e.name.clone(), e.gender, "auto"))
            };
            let Some((name, gender, source)) = launched.or_else(kept).or_else(|| {
                self.fresh(&w.key, &held, &rejected)
                    .map(|(n, g)| (n, g, "auto"))
            }) else {
                continue;
            };
            self.agents.insert(
                w.key.clone(),
                Entry {
                    name,
                    gender,
                    team: w.team.clone(),
                    role: w.role.clone(),
                    source: source.into(),
                    reason: None,
                    assigned_ms: now,
                    team_explicit: false,
                },
            );
            changed = true;
        }
        changed
    }

    /// The best free name for `key`: the highest score, ties broken by a
    /// hash of the key so the same agent always gets the same name. Past
    /// the list, the same order with a surname: always a name.
    fn fresh(
        &self,
        key: &str,
        held: &BTreeSet<String>,
        rejected: &BTreeSet<String>,
    ) -> Option<(String, Gender)> {
        let seed = fnv(key);
        // The weights once, and each name scored once from its features
        // worked out in advance: the sort compares keys, it does not
        // rebuild them.
        let weights = self.weights();
        let mut ranked: Vec<Ranked<'_>> = names()
            .zip(listed_keys())
            .filter(|(_, k)| !rejected.contains(&k.lower))
            .map(|((n, g), k)| {
                let score: i32 = k
                    .features
                    .iter()
                    .map(|f| weights.get(f).copied().unwrap_or(0))
                    .sum();
                (std::cmp::Reverse((score, k.hash ^ seed)), n, g)
            })
            .collect();
        ranked.sort_by_key(|(key, _, _)| *key);
        (1..=SUFFIX_MAX).find_map(|round| {
            ranked.iter().find_map(|(_, n, g)| {
                let name = suffixed(n, round);
                (!held.contains(&name.to_lowercase())).then_some((name, *g))
            })
        })
    }

    /// The operator renamed an agent (double click). The new name is free text,
    /// cleaned; it must not be held by another live agent.
    pub fn rename(
        &mut self,
        key: &str,
        raw: &str,
        live: &BTreeSet<&str>,
        now: u64,
    ) -> Result<String, &'static str> {
        let name = clean_name(raw).ok_or("empty_name")?;
        if self.agents.iter().any(|(k, e)| {
            k != key && live.contains(k.as_str()) && e.name.eq_ignore_ascii_case(&name)
        }) {
            return Err("name_taken");
        }
        let gender = listed(&name)
            .map(|(_, g)| g)
            .or_else(|| self.agents.get(key).map(|e| e.gender))
            .unwrap_or(if name.ends_with('a') {
                Gender::F
            } else {
                Gender::M
            });
        let old = self.agents.get(key).cloned();
        if let Some(old) = &old {
            self.push_vote(Vote {
                ts_ms: now,
                agent: key.into(),
                name: old.name.clone(),
                vote: "rename".into(),
                to: Some(name.clone()),
            });
        }
        let entry = Entry {
            name: name.clone(),
            gender,
            team: old.as_ref().and_then(|e| e.team.clone()),
            role: old.as_ref().and_then(|e| e.role.clone()),
            source: "renamed".into(),
            reason: None,
            assigned_ms: now,
            team_explicit: old.as_ref().is_some_and(|e| e.team_explicit),
        };
        self.agents.insert(key.to_string(), entry);
        Ok(name)
    }

    /// A 👍 or 👎 on an agent's name. A 👎 also gives the agent a new name,
    /// and the old one is never given again.
    pub fn vote(&mut self, key: &str, up: bool, live: &BTreeSet<&str>, now: u64) -> Option<String> {
        let entry = self.agents.get(key)?.clone();
        self.push_vote(Vote {
            ts_ms: now,
            agent: key.into(),
            name: entry.name.clone(),
            vote: if up { "up" } else { "down" }.into(),
            to: None,
        });
        if up {
            return Some(entry.name);
        }
        let held = self.held(live);
        let (name, gender) = self.fresh(key, &held, &self.rejected())?;
        self.agents.insert(
            key.to_string(),
            Entry {
                name: name.clone(),
                gender,
                source: "auto".into(),
                assigned_ms: now,
                ..entry
            },
        );
        Some(name)
    }

    /// A session picked a name from the list (`harbard name`).
    pub fn pick(
        &mut self,
        key: &str,
        raw: &str,
        reason: Option<&str>,
        live: &BTreeSet<&str>,
        now: u64,
    ) -> Result<String, &'static str> {
        let (name, gender) = listed(raw).ok_or("not_in_list")?;
        if self.rejected().contains(&name.to_lowercase()) {
            return Err("name_rejected");
        }
        if self.agents.iter().any(|(k, e)| {
            k != key && live.contains(k.as_str()) && e.name.eq_ignore_ascii_case(name)
        }) {
            return Err("name_taken");
        }
        let old = self.agents.get(key).cloned();
        self.agents.insert(
            key.to_string(),
            Entry {
                name: name.to_string(),
                gender,
                team: old.as_ref().and_then(|e| e.team.clone()),
                role: old.as_ref().and_then(|e| e.role.clone()),
                source: "picked".into(),
                reason: reason
                    .map(|r| {
                        r.lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .chars()
                            .take(140)
                            .collect()
                    })
                    .filter(|r: &String| !r.is_empty()),
                assigned_ms: now,
                team_explicit: old.as_ref().is_some_and(|e| e.team_explicit),
            },
        );
        Ok(name.to_string())
    }

    /// `harbard name --team <team>`: the team said outright. `None` when the
    /// agent has no name yet to carry it.
    pub fn set_team(&mut self, key: &str, team: &str) -> Option<()> {
        let team = team.trim().trim_start_matches('.');
        let e = self.agents.get_mut(key)?;
        e.team = (!team.is_empty()).then(|| team.to_string());
        e.team_explicit = e.team.is_some();
        Some(())
    }

    fn push_vote(&mut self, v: Vote) {
        self.votes.push(v);
        if self.votes.len() > VOTES_MAX {
            let drop = self.votes.len() - VOTES_MAX;
            self.votes.drain(..drop);
        }
    }
}

/// Rounds of the list before giving up: 64 names × 20.
const SUFFIX_MAX: u32 = 20;

/// The listed name under a surnamed one: "Nora Walsh" → "Nora".
fn base_of(name: &str) -> &str {
    name.rsplit_once(' ')
        .filter(|(b, surname)| listed(b).is_some() && SURNAMES.contains(surname))
        .map_or(name, |(b, _)| b)
}

/// `name` in round `round`: itself first, then with a surname of its own
/// order ("Nora Walsh", "Nora Keller"…). `SUFFIX_MAX` is below the number of
/// surnames, so no two rounds of one name repeat.
fn suffixed(name: &str, round: u32) -> String {
    match round {
        1 => name.to_string(),
        r => {
            let at = (fnv(&name.to_lowercase()) as usize + r as usize) % SURNAMES.len();
            format!("{name} {}", SURNAMES[at])
        }
    }
}

/// A name's score under `weights` ([`Registry::score`]).
fn score_in(weights: &BTreeMap<String, i32>, name: &str) -> i32 {
    features(name, None)
        .iter()
        .map(|f| weights.get(f).copied().unwrap_or(0))
        .sum()
}

/// FNV-1a: stable across builds and machines.
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.to_lowercase().as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(key: &str, team: &str, role: &str, live: bool, first: u64) -> Wanted {
        Wanted {
            key: key.into(),
            team: Some(team.into()),
            role: Some(role.into()),
            live,
            first_ms: first,
            ..Default::default()
        }
    }

    fn unique(r: &Registry, keys: &[&str]) -> bool {
        let names: BTreeSet<String> = keys
            .iter()
            .map(|k| r.agents[*k].name.to_lowercase())
            .collect();
        names.len() == keys.len()
    }

    /// AC4: acme's subagents carry a team and no role. A role of None
    /// matched None, so eleven of them took one finished agent's name:
    /// "Sarah, Sarah, Sarah".
    #[test]
    fn agents_without_a_role_never_inherit_a_name() {
        let mut r = Registry::default();
        let mut wanted = vec![Wanted {
            key: "old".into(),
            team: Some("acme".into()),
            live: true,
            first_ms: 1,
            ..Default::default()
        }];
        r.assign(&wanted, &BTreeSet::new(), 1);
        wanted[0].live = false;
        let keys: Vec<String> = (0..11).map(|i| format!("sub{i}")).collect();
        for (i, k) in keys.iter().enumerate() {
            wanted.push(Wanted {
                key: k.clone(),
                team: Some("acme".into()),
                live: true,
                first_ms: 10 + i as u64,
                ..Default::default()
            });
        }
        r.assign(&wanted, &BTreeSet::new(), 2);
        let mut all: Vec<&str> = keys.iter().map(String::as_str).collect();
        all.push("old");
        assert!(unique(&r, &all), "{:?}", r.agents);
    }

    /// AC4: a teammate launched as "Laura" kept that name outside the
    /// registry, which then gave "Laura" to another agent as well.
    #[test]
    fn a_launched_workshop_name_goes_through_the_registry() {
        let mut r = Registry::default();
        r.assign(&[w("auto", "t", "x", true, 1)], &BTreeSet::new(), 1);
        let taken = r.agents["auto"].name.clone();
        let free = names()
            .map(|(n, _)| n)
            .find(|n| !n.eq_ignore_ascii_case(&taken))
            .unwrap();
        let launched = |key: &str, own: &str, first: u64| Wanted {
            own: Some(own.into()),
            ..w(key, "harbard", "constructor", true, first)
        };
        let wanted = [
            w("auto", "t", "x", true, 1),
            launched("clash", &taken.to_uppercase(), 2),
            launched("mine", free, 3),
        ];
        r.assign(&wanted, &BTreeSet::new(), 2);
        assert_eq!(r.agents["mine"].name, free, "its own name, registered");
        assert_eq!(r.agents["mine"].source, "launched");
        assert_eq!(
            r.agents["clash"].name,
            suffixed(&taken, 2),
            "held by another: its own name, with a surname"
        );
        assert!(unique(&r, &["auto", "clash", "mine"]), "{:?}", r.agents);
        // And the registry now holds it: nobody else gets it.
        let mut more: Vec<Wanted> = wanted.to_vec();
        more.extend((0..70).map(|i| w(&format!("n{i}"), "t", "y", true, 10 + i)));
        r.assign(&more, &BTreeSet::new(), 3);
        assert!(
            r.agents
                .iter()
                .all(|(k, e)| k == "mine" || !e.name.eq_ignore_ascii_case(free))
        );
    }

    fn entry(name: &str, source: &str, at: u64) -> Entry {
        Entry {
            name: name.into(),
            gender: Gender::F,
            team: Some("acme".into()),
            role: None,
            source: source.into(),
            reason: None,
            assigned_ms: at,
            team_explicit: false,
        }
    }

    fn shown(key: &str, first: u64, own: Option<&str>) -> Wanted {
        Wanted {
            key: key.into(),
            team: Some("acme".into()),
            live: true,
            first_ms: first,
            own: own.map(str::to_string),
            ..Default::default()
        }
    }

    /// AC4 on the operator's own names.json, which already holds repeats (11
    /// "Sarah"): read, the oldest agent keeps the name and the others get
    /// a free name of the list, ids untouched; a second read changes
    /// nothing. A suffix is only for when the list runs out.
    #[test]
    fn a_names_json_with_repeats_is_settled_on_read() {
        let mut r = Registry::default();
        r.agents.insert("young".into(), entry("Sarah", "auto", 1));
        r.agents.insert("oldest".into(), entry("Sarah", "auto", 2));
        r.agents.insert("middle".into(), entry("sarah", "auto", 3));
        // Not shown: its repeat is nobody's business.
        r.agents.insert("gone".into(), entry("Sarah", "auto", 0));
        let wanted = [
            shown("young", 30, None),
            shown("oldest", 10, None),
            shown("middle", 20, None),
        ];
        assert!(r.needs_settle(&wanted, &BTreeSet::new(), 9));
        assert!(r.settle(&wanted, &BTreeSet::new(), 9));
        let name = |k: &str| r.agents[k].name.clone();
        assert_eq!(name("oldest"), "Sarah", "the oldest keeps it");
        for k in ["middle", "young"] {
            assert!(
                listed(&name(k)).is_some(),
                "a free name of the list, no suffix: {}",
                name(k)
            );
            assert!(!name(k).eq_ignore_ascii_case("sarah"));
        }
        assert_ne!(name("middle"), name("young"));
        assert_eq!(name("gone"), "Sarah", "an agent not shown is not touched");
        assert_eq!(r.agents.len(), 4, "no id added or dropped");
        assert!(!r.needs_settle(&wanted, &BTreeSet::new(), 10));
        assert!(!r.settle(&wanted, &BTreeSet::new(), 10), "settled once");
    }

    /// The installed AC4 gave every launched teammate a suffix: dozens of
    /// old Codex sessions, still shown under Inactive, held "Clara" as
    /// `auto`. A launched name beats an auto one: the auto agent moves to a
    /// free name, the teammate reads "Clara", and "Emma Walsh" (launched,
    /// numbered because of an auto) goes back to "Emma".
    #[test]
    fn a_launched_name_wins_over_auto_holders_and_drops_its_suffix() {
        let mut r = Registry::default();
        for i in 0..5 {
            r.agents
                .insert(format!("codex{i}"), entry("Clara", "auto", i));
        }
        r.agents
            .insert("old-pulley".into(), entry("Emma", "auto", 1));
        r.agents
            .insert("emma".into(), entry("Emma Walsh", "launched", 50));
        r.agents
            .insert("auto-lima".into(), entry("Sarah", "auto", 60));
        let mut wanted: Vec<Wanted> = (0..5)
            .map(|i| shown(&format!("codex{i}"), i, None))
            .collect();
        wanted.push(shown("old-pulley", 1, None));
        wanted.push(shown("emma", 50, Some("Emma")));
        wanted.push(shown("light", 70, Some("Clara")));
        // Launched as "Laura", named "Sarah" by the old rules: its own name.
        wanted.push(shown("auto-lima", 60, Some("Laura")));
        assert!(r.settle(&wanted, &BTreeSet::new(), 9));
        let name = |k: &str| r.agents[k].name.clone();
        assert_eq!(name("emma"), "Emma");
        assert_eq!(name("light"), "Clara");
        assert_eq!(name("auto-lima"), "Laura");
        assert_eq!(r.agents["light"].source, "launched");
        assert_eq!(r.agents["auto-lima"].source, "launched");
        let keys: Vec<&str> = wanted.iter().map(|w| w.key.as_str()).collect();
        assert!(unique(&r, &keys), "{:?}", r.agents);
        for k in [
            "codex0",
            "codex1",
            "codex2",
            "codex3",
            "codex4",
            "old-pulley",
        ] {
            assert!(
                listed(&name(k)).is_some(),
                "an auto moves to a free listed name: {}",
                name(k)
            );
        }
        assert!(!r.needs_settle(&wanted, &BTreeSet::new(), 10));
    }

    /// Two teammates launched with one name: the older keeps it clean.
    /// And the operator's rename is never taken away by a launched name.
    #[test]
    fn between_launched_names_the_oldest_wins_and_a_rename_stays() {
        let mut r = Registry::default();
        r.agents.insert("mine".into(), entry("Ruth", "renamed", 1));
        let wanted = [
            shown("mine", 1, None),
            shown("t-new", 30, Some("Ruth")),
            shown("l-old", 10, Some("Laura")),
            shown("l-new", 20, Some("Laura")),
        ];
        r.settle(&wanted, &BTreeSet::new(), 9);
        let name = |k: &str| r.agents[k].name.clone();
        assert_eq!(name("mine"), "Ruth", "their rename stays");
        assert_eq!(name("t-new"), suffixed("Ruth", 2));
        assert_eq!(name("l-old"), "Laura");
        assert_eq!(name("l-new"), suffixed("Laura", 2));

        // Today's Iris at work, an older one finished: today's reads
        // "Iris".
        let mut r = Registry::default();
        let mut done = shown("b-old", 1, Some("Iris"));
        done.live = false;
        let wanted = [done, shown("b-today", 50, Some("Iris"))];
        r.settle(&wanted, &BTreeSet::new(), 9);
        assert_eq!(r.agents["b-today"].name, "Iris");
        assert_eq!(r.agents["b-old"].name, suffixed("Iris", 2));
        // Both finished later: no swap back.
        let mut wanted = wanted;
        wanted[1].live = false;
        assert!(!r.settle(&wanted, &BTreeSet::new(), 10));
        assert_eq!(r.agents["b-today"].name, "Iris");
    }

    /// AC4: past the list, "Sarah Walsh", in the same style; never a raw
    /// id, and never a name twice.
    #[test]
    fn past_the_list_names_take_a_surname() {
        let mut r = Registry::default();
        let total = name_count() + 30;
        let wanted: Vec<Wanted> = (0..total)
            .map(|i| w(&format!("a{i:03}"), "t", &format!("r{i}"), true, i as u64))
            .collect();
        r.assign(&wanted, &BTreeSet::new(), 1);
        let keys: Vec<&str> = wanted.iter().map(|w| w.key.as_str()).collect();
        assert_eq!(r.agents.len(), total, "every agent has a name");
        assert!(unique(&r, &keys));
        let suffixed: Vec<&str> = r
            .agents
            .values()
            .map(|e| e.name.as_str())
            .filter(|n| listed(n).is_none())
            .collect();
        assert_eq!(suffixed.len(), total - name_count());
        for n in &suffixed {
            let (base, surname) = n.rsplit_once(' ').unwrap();
            assert!(listed(base).is_some(), "{n}");
            assert!(SURNAMES.contains(&surname), "{n}");
        }
    }

    fn live<'a>(keys: &[&'a str]) -> BTreeSet<&'a str> {
        keys.iter().copied().collect()
    }

    #[test]
    fn every_name_is_listed_once_and_can_be_a_callsign() {
        assert!(name_count() >= 400);
        let unique: BTreeSet<String> = names().map(|(n, _)| n.to_lowercase()).collect();
        assert_eq!(unique.len(), name_count());
        for (n, _) in names() {
            assert!((3..=12).contains(&n.len()), "{n}");
            assert!(n.chars().all(|c| c.is_ascii_alphabetic()), "{n}");
            assert!(!crate::alias::is_generic(n), "{n}");
        }
        assert_eq!(listed("nora").map(|(_, g)| g), Some(Gender::F));
        assert_eq!(listed("OWEN").map(|(_, g)| g), Some(Gender::M));
        assert!(SURNAMES.len() > SUFFIX_MAX as usize);
    }

    #[test]
    fn a_name_from_the_old_list_is_replaced_and_a_chosen_one_stays() {
        let entry = |name: &str, source: &str| Entry {
            name: name.into(),
            gender: Gender::F,
            team: None,
            role: None,
            source: source.into(),
            reason: None,
            assigned_ms: 1,
            team_explicit: false,
        };
        let mut r = Registry::default();
        r.agents.insert("old".into(), entry("Tuerquita", "auto"));
        r.agents
            .insert("old2".into(), entry("Mecha II", "launched"));
        r.agents
            .insert("kept".into(), entry("Tuerquita", "renamed"));
        r.agents.insert("new".into(), entry("Nora", "auto"));
        let wanted: Vec<Wanted> = ["old", "old2", "kept", "new"]
            .iter()
            .map(|k| Wanted {
                key: (*k).into(),
                live: true,
                first_ms: 1,
                ..Default::default()
            })
            .collect();
        assert!(r.settle(&wanted, &BTreeSet::new(), 2));
        assert!(listed(&r.agents["old"].name).is_some());
        assert!(listed(&r.agents["old2"].name).is_some());
        assert_eq!(r.agents["kept"].name, "Tuerquita");
        assert_eq!(r.agents["new"].name, "Nora");
    }

    #[test]
    fn a_second_holder_takes_a_surname_and_keeps_its_gender() {
        let name = suffixed("Nora", 2);
        let (first, surname) = name.split_once(' ').unwrap();
        assert_eq!(first, "Nora");
        assert!(SURNAMES.contains(&surname));
        assert_eq!(base_of(&name), "Nora");
        let rounds: BTreeSet<String> = (2..=SUFFIX_MAX).map(|r| suffixed("Nora", r)).collect();
        assert_eq!(rounds.len(), SUFFIX_MAX as usize - 1);
    }

    #[test]
    fn a_known_role_is_coded_and_free_text_is_not() {
        assert_eq!(
            role_line(Some("revisor"), Some("protocol")).as_deref(),
            Some("reviewer · protocol")
        );
        assert_eq!(
            role_line(Some("constructor"), Some("protocol")).as_deref(),
            Some("builder · protocol")
        );
        let (code, values) =
            role_line_coded(Some("revisor 2"), Some("protocol"), Gender::F).unwrap();
        assert_eq!(code, "role_line");
        assert_eq!(
            serde_json::to_value(&values).unwrap(),
            serde_json::json!({"role": "reviewer", "gender": "f", "number": "2", "team": "protocol"})
        );
        assert_eq!(
            role_line_coded(Some("the ledger"), Some("x"), Gender::M),
            None
        );
        assert_eq!(
            role_line(Some("the ledger"), Some("x")).as_deref(),
            Some("the ledger · x")
        );
    }

    #[test]
    fn a_new_agent_does_not_take_a_name_a_session_goes_by() {
        let wanted = [w("new", "t", "r", true, 1)];
        let mut unaware = Registry::default();
        unaware.settle(&wanted, &BTreeSet::new(), 1);
        let natural = unaware.agents["new"].name.clone();

        let taken = BTreeSet::from([natural.to_lowercase()]);
        let mut r = Registry::default();
        r.settle(&wanted, &taken, 1);
        assert_ne!(r.agents["new"].name, natural);
    }

    #[test]
    fn a_new_agent_launched_with_a_session_name_takes_a_surname_and_keeps_it() {
        let wanted = [Wanted {
            own: Some("Nora".into()),
            ..w("new", "t", "r", true, 1)
        }];
        let taken = BTreeSet::from(["nora".to_string()]);
        let mut r = Registry::default();
        r.settle(&wanted, &taken, 1);
        let name = r.agents["new"].name.clone();
        assert!(name.starts_with("Nora ") && name != "Nora", "{name}");
        assert!(!r.settle(&wanted, &taken, 2), "the surname is stable");
    }

    #[test]
    fn a_name_already_given_stays_when_a_session_goes_by_it() {
        let mut r = Registry::default();
        r.agents.insert("old".into(), entry("Oscar", "auto", 1));
        let wanted = [w("old", "t", "r", false, 1)];
        let taken = BTreeSet::from(["oscar".to_string()]);
        assert!(!r.settle(&wanted, &taken, 2));
        assert_eq!(r.agents["old"].name, "Oscar");
    }

    #[test]
    fn names_are_unique_among_live_agents_and_stable_per_id() {
        let mut r = Registry::default();
        let agents: Vec<Wanted> = (0..40)
            .map(|i| w(&format!("a{i}"), "t", &format!("r{i}"), true, i))
            .collect();
        assert!(r.assign(&agents, &BTreeSet::new(), 1));
        let names: BTreeSet<String> = r.agents.values().map(|e| e.name.to_lowercase()).collect();
        assert_eq!(names.len(), 40, "unique among live agents");
        let first = r.agents["a7"].name.clone();
        // Assigning again changes nothing; a reload reads the same.
        assert!(!r.assign(&agents, &BTreeSet::new(), 2));
        let home = crate::test_util::TempHome::new();
        r.save(home.path()).unwrap();
        assert_eq!(Registry::load(home.path()).agents["a7"].name, first);
        // The same id in a fresh registry gets the same name too.
        let mut again = Registry::default();
        again.assign(&agents, &BTreeSet::new(), 9);
        assert_eq!(again.agents["a7"].name, first);
    }

    #[test]
    fn a_finished_agent_still_shown_keeps_its_name_to_itself() {
        // A finished agent and a live one got the same
        // name. Shown agents never share one, finished or not.
        let mut r = Registry::default();
        let wanted = [
            w("done", "notas", "indice", false, 1),
            w("live", "robots", "integ", true, 2),
        ];
        r.assign(&wanted, &BTreeSet::new(), 1);
        assert_ne!(r.agents["done"].name, r.agents["live"].name);
    }

    #[test]
    fn an_agent_keeps_its_name_across_rounds_of_the_same_team_and_role() {
        let mut r = Registry::default();
        r.assign(
            &[w("round1", "protocol", "constructor", true, 1)],
            &BTreeSet::new(),
            1,
        );
        let name = r.agents["round1"].name.clone();
        // Round 1 finished but is still shown: two rows never read the
        // same (AC4), so round 2 gets a name of its own.
        let mut shown = r.clone();
        shown.assign(
            &[
                w("round1", "protocol", "constructor", false, 1),
                w("round2", "protocol", "constructor", true, 2),
            ],
            &BTreeSet::new(),
            2,
        );
        assert_ne!(shown.agents["round2"].name, name);
        // Round 1 left the list; round 2 of the same team and role takes
        // the name.
        r.assign(
            &[w("round2", "protocol", "constructor", true, 2)],
            &BTreeSet::new(),
            2,
        );
        assert_eq!(r.agents["round2"].name, name);
        // A different role does not.
        r.assign(
            &[
                w("rev", "protocol", "revisor", true, 3),
                w("round2", "protocol", "constructor", true, 2),
            ],
            &BTreeSet::new(),
            3,
        );
        assert_ne!(r.agents["rev"].name, name);
    }

    #[test]
    fn a_thumbs_down_name_is_never_given_again() {
        let mut r = Registry::default();
        r.assign(&[w("a", "t", "x", true, 1)], &BTreeSet::new(), 1);
        let bad = r.agents["a"].name.clone();
        let new = r.vote("a", false, &live(&["a"]), 2).unwrap();
        assert_ne!(new, bad);
        assert!(r.rejected().contains(&bad.to_lowercase()));
        // Neither a fresh agent nor a later round of the same role gets it.
        let many: Vec<Wanted> = (0..63)
            .map(|i| w(&format!("b{i}"), "t", "x", true, 10 + i))
            .collect();
        r.assign(&many, &BTreeSet::new(), 3);
        assert!(
            r.agents
                .values()
                .all(|e| !e.name.eq_ignore_ascii_case(&bad))
        );
        assert_eq!(r.pick("s", &bad, None, &live(&[]), 4), Err("name_rejected"));
    }

    #[test]
    fn a_rename_persists_and_teaches_the_generator() {
        let home = crate::test_util::TempHome::new();
        Registry::update(home.path(), |r| {
            r.assign(
                &[w("a", "t", "x", true, 1), w("b", "t", "y", true, 2)],
                &BTreeSet::new(),
                1,
            );
        })
        .unwrap();
        let taken = Registry::load(home.path()).agents["b"].name.clone();
        let err = Registry::update(home.path(), |r| {
            r.rename("a", &taken, &live(&["a", "b"]), 2)
        })
        .unwrap();
        assert_eq!(err, Err("name_taken"));
        let ok = Registry::update(home.path(), |r| {
            r.rename("a", "  clara ", &live(&["a", "b"]), 3)
        })
        .unwrap();
        assert_eq!(ok, Ok("Clara".into()));
        let back = Registry::load(home.path());
        assert_eq!(
            (
                back.agents["a"].name.as_str(),
                back.agents["a"].source.as_str()
            ),
            ("Clara", "renamed")
        );
        assert_eq!(back.votes.last().unwrap().vote, "rename");
        // Renamed towards a long name in -a of the mechanism category: those
        // features now score above their opposites.
        assert!(back.score("Lucy") > back.score("Ian"));
    }

    #[test]
    fn a_session_picks_from_the_list_with_its_reason() {
        let mut r = Registry::default();
        assert_eq!(
            r.pick(
                "session:s",
                "Hazel",
                Some("I guide the team.\nsecond line"),
                &live(&[]),
                1
            ),
            Ok("Hazel".into())
        );
        assert_eq!(
            r.agents["session:s"].reason.as_deref(),
            Some("I guide the team.")
        );
        assert_eq!(
            r.pick("session:t", "Nombre Inventado", None, &live(&[]), 1),
            Err("not_in_list")
        );
        assert_eq!(
            r.pick("session:t", "hazel", None, &live(&["session:s"]), 1),
            Err("name_taken")
        );
    }
}

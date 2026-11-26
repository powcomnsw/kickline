//! Replays every recording in tests/recordings through the section detector.
//!
//! Drop a file made by Diagnostics > Record a session into that folder and
//! this test scores it against the marks pressed while it was made. A recording
//! that reproduces a misread is added with a `.known-bad` name part and only
//! has to replay without panicking and give the same answer twice.

use lumen_core::recording::Recording;
use lumen_core::Genre;

#[test]
fn recordings_replay_the_same_every_time() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/recordings");
    let mut found = 0;
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let r = Recording::from_json(&std::fs::read_to_string(&p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let genre = Genre::ALL.iter().copied().find(|g| g.name() == r.genre).unwrap_or(Genre::House);
        let a = r.replay(genre.profile());
        assert_eq!(a, r.replay(genre.profile()), "{}", p.display());
        let (ok, n) = r.score(&a, 8.0, 8.0);
        println!("{}: {ok}/{n} frames match the marks", p.display());
        if !p.to_string_lossy().contains(".known-bad") && n > 0 {
            assert!(ok as f64 / n as f64 > 0.6, "{}: only {ok}/{n} match the marks", p.display());
        }
        found += 1;
    }
    println!("{found} recording(s) replayed");
}

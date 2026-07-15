use voxd::cache::AudioCache;
use voxd::Settings;

#[test]
fn key_is_stable_and_sensitive() {
    let tmp = tempfile::tempdir().unwrap();
    let c = AudioCache::new(tmp.path().to_path_buf(), true, 512).unwrap();
    let s = Settings::default();
    let k1 = c.key("hello", "voice1", "model", "mp3_44100_128", &s);
    let k2 = c.key("hello", "voice1", "model", "mp3_44100_128", &s);
    let k3 = c.key("world", "voice1", "model", "mp3_44100_128", &s);
    let k4 = c.key("hello", "voice2", "model", "mp3_44100_128", &s);
    assert_eq!(k1, k2);
    assert_ne!(k1, k3);
    assert_ne!(k1, k4);
}

#[test]
fn put_then_get_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let c = AudioCache::new(tmp.path().to_path_buf(), true, 512).unwrap();
    c.put("abc", b"RIFF").unwrap();
    assert_eq!(c.get("abc"), Some(b"RIFF".to_vec()));
    assert_eq!(c.get("missing"), None);
}

#[test]
fn disabled_cache_never_hits() {
    let tmp = tempfile::tempdir().unwrap();
    let c = AudioCache::new(tmp.path().to_path_buf(), false, 512).unwrap();
    c.put("abc", b"RIFF").unwrap(); // put is a no-op when disabled
    assert_eq!(c.get("abc"), None);
}

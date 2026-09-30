use super::*;

const ID: &str = "dQw4w9WgXcQ";

fn ok(raw: &str) -> YoutubeRef {
    normalize_link(raw).unwrap_or_else(|e| panic!("{raw:?} sollte gelten, war aber {e:?}"))
}

fn err(raw: &str) -> LinkError {
    normalize_link(raw).expect_err(&format!("{raw:?} sollte abgewiesen werden"))
}

#[test]
fn a_watch_link_yields_the_video_id() {
    let r = ok("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    assert_eq!(r.video_id, ID);
    assert_eq!(r.start_s, None);
    assert_eq!(
        r.canonical_url(),
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    );
}

#[test]
fn every_known_form_of_a_video_link_is_understood() {
    let forms = [
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "http://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "https://youtube.com/watch?v=dQw4w9WgXcQ",
        "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
        "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
        "https://youtu.be/dQw4w9WgXcQ",
        "https://www.youtube.com/shorts/dQw4w9WgXcQ",
        "https://www.youtube.com/embed/dQw4w9WgXcQ",
        "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
        "https://www.youtube.com/live/dQw4w9WgXcQ",
        "https://www.youtube.com/v/dQw4w9WgXcQ",
        // Parameter in beliebiger Reihenfolge, Verfolgung (si) und Wiedergabeliste.
        "https://www.youtube.com/watch?feature=share&v=dQw4w9WgXcQ&si=AbCdEf123",
        "https://youtu.be/dQw4w9WgXcQ?si=AbCdEf123",
        // Ohne Schema, mit Leerraum und in Anfuehrungszeichen, wie aus der Zwischenablage.
        "www.youtube.com/watch?v=dQw4w9WgXcQ",
        "youtu.be/dQw4w9WgXcQ",
        "  https://youtu.be/dQw4w9WgXcQ \n",
        "<https://youtu.be/dQw4w9WgXcQ>",
        "\"https://youtu.be/dQw4w9WgXcQ\"",
        // Gross-/Kleinschreibung des Hosts.
        "HTTPS://WWW.YOUTUBE.COM/watch?v=dQw4w9WgXcQ",
    ];
    for raw in forms {
        assert_eq!(ok(raw).video_id, ID, "{raw}");
    }
}

#[test]
fn the_start_time_is_read_from_t_and_start() {
    let cases = [
        ("https://youtu.be/dQw4w9WgXcQ?t=90", Some(90)),
        ("https://youtu.be/dQw4w9WgXcQ?t=90s", Some(90)),
        (
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1m30s",
            Some(90),
        ),
        (
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1h2m3s",
            Some(3723),
        ),
        (
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=2M",
            Some(120),
        ),
        (
            "https://www.youtube.com/embed/dQw4w9WgXcQ?start=45",
            Some(45),
        ),
        ("https://www.youtube.com/watch?v=dQw4w9WgXcQ#t=30", Some(30)),
        // Unsinn wird ignoriert, der Link gilt trotzdem.
        ("https://youtu.be/dQw4w9WgXcQ?t=abc", None),
        ("https://youtu.be/dQw4w9WgXcQ?t=-5", None),
        ("https://youtu.be/dQw4w9WgXcQ?t=99999999999", None),
        ("https://youtu.be/dQw4w9WgXcQ?t=", None),
        ("https://youtu.be/dQw4w9WgXcQ?t=3m1h", None),
    ];
    for (raw, want) in cases {
        let r = ok(raw);
        assert_eq!(r.video_id, ID, "{raw}");
        assert_eq!(r.start_s, want, "{raw}");
    }
}

#[test]
fn the_canonical_url_drops_tracking_and_list_parameters() {
    let r = ok("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL123&index=3&si=ZZZ&t=10");
    let url = r.canonical_url();
    assert_eq!(url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    assert!(!url.contains("si="));
    assert!(!url.contains("list="));
}

#[test]
fn a_playlist_link_gets_its_own_message() {
    for raw in [
        "https://www.youtube.com/playlist?list=PLabcdefghij",
        "https://www.youtube.com/watch?list=PLabcdefghij",
        "https://youtube.com/embed/videoseries?list=PLabcdefghij",
        "https://music.youtube.com/playlist?list=OLAK5uy_abc",
    ] {
        assert_eq!(err(raw), LinkError::Playlist, "{raw}");
    }
    assert_eq!(LinkError::Playlist.code(), "youtube_playlist");
    assert!(LinkError::Playlist.to_string().contains("Playlists"));
}

#[test]
fn a_channel_link_gets_its_own_message() {
    for raw in [
        "https://www.youtube.com/@wolffappliedai",
        "https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv",
        "https://www.youtube.com/c/WolffAppliedAI",
        "https://www.youtube.com/user/someone",
    ] {
        assert_eq!(err(raw), LinkError::Channel, "{raw}");
    }
    assert_eq!(LinkError::Channel.code(), "youtube_channel");
}

#[test]
fn a_youtube_page_without_a_video_is_refused() {
    for raw in [
        "https://www.youtube.com/",
        "https://www.youtube.com",
        "https://www.youtube.com/results?search_query=lastgang",
        "https://www.youtube.com/feed/subscriptions",
        "https://www.youtube.com/watch",
        "https://youtu.be/",
        "https://www.youtube.com/shorts/",
    ] {
        assert_eq!(err(raw), LinkError::NoVideo, "{raw}");
    }
}

#[test]
fn a_malformed_video_id_is_refused() {
    for raw in [
        "https://www.youtube.com/watch?v=tooshort",
        "https://www.youtube.com/watch?v=dQw4w9WgXcQextra",
        "https://youtu.be/dQw4w9WgXc!",
        "https://youtu.be/dQw4w9WgXc%20",
        "https://www.youtube.com/watch?v=dQw4w9WgXc.",
        "https://www.youtube.com/watch?v=<script>alert(1)</script>",
        "https://www.youtube.com/watch?v=",
    ] {
        assert_eq!(err(raw), LinkError::BadVideoId, "{raw}");
    }
}

#[test]
fn video_ids_are_exactly_eleven_url_safe_characters() {
    assert!(valid_video_id("dQw4w9WgXcQ"));
    assert!(valid_video_id("-_-_-_-_-_-"));
    assert!(valid_video_id("00000000000"));
    assert!(!valid_video_id(""));
    assert!(!valid_video_id("dQw4w9WgXc"));
    assert!(!valid_video_id("dQw4w9WgXcQQ"));
    assert!(!valid_video_id("dQw4w9WgXc "));
    assert!(!valid_video_id("dQw4w9WgXcé"));
}

#[test]
fn other_sites_and_lookalike_hosts_are_not_youtube() {
    for raw in [
        "https://vimeo.com/123456789",
        "https://example.com/watch?v=dQw4w9WgXcQ",
        "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
        "https://www.youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
        "https://evilyoutube.com/watch?v=dQw4w9WgXcQ",
        "https://youtu.be.evil.example/dQw4w9WgXcQ",
        "https://notyoutu.be/dQw4w9WgXcQ",
        "https://youtube.com@evil.example/watch?v=dQw4w9WgXcQ",
        "https://www.youtube.com:8443/watch?v=dQw4w9WgXcQ",
        "https://user:pw@www.youtube.com/watch?v=dQw4w9WgXcQ",
        // Andere Unterdomaene von youtube.com: nicht auf der Liste.
        "https://studio.youtube.com/video/dQw4w9WgXcQ",
        // Schriftzeichen, die wie lateinische aussehen.
        "https://www.yоutube.com/watch?v=dQw4w9WgXcQ",
        "ftp://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "file:///C:/Users/x/watch?v=dQw4w9WgXcQ",
        "javascript:alert(1)",
    ] {
        assert_eq!(err(raw), LinkError::NotYoutube, "{raw}");
    }
}

#[test]
fn text_that_is_no_address_is_refused() {
    for raw in [
        "",
        "   ",
        "\n\t",
        "hallo welt",
        "dQw4w9WgXcQ",
        "https://",
        "http:///x",
    ] {
        let e = err(raw);
        assert!(
            matches!(
                e,
                LinkError::Empty | LinkError::NotAnAddress | LinkError::NotYoutube
            ),
            "{raw:?} -> {e:?}"
        );
    }
    assert_eq!(err(""), LinkError::Empty);
    assert_eq!(err("   "), LinkError::Empty);
}

#[test]
fn control_characters_and_huge_input_are_refused() {
    assert_eq!(
        err("https://youtu.be/dQw4w9WgXcQ\u{0}"),
        LinkError::NotAnAddress
    );
    assert_eq!(
        err("https://youtu.be/dQw4w9WgXcQ\u{7}x"),
        LinkError::NotAnAddress
    );
    // Zeilenumbruch mitten im Link: keine zwei Links zu einem machen.
    assert_eq!(
        err("https://youtu.be/dQw4w9WgXcQ\nhttps://youtu.be/aaaaaaaaaaa"),
        LinkError::NotAnAddress
    );
    let long = format!(
        "https://youtu.be/dQw4w9WgXcQ?x={}",
        "a".repeat(MAX_LINK_CHARS)
    );
    assert_eq!(err(&long), LinkError::TooLong);
}

#[test]
fn error_codes_are_stable_and_messages_are_german() {
    let all = [
        LinkError::Empty,
        LinkError::TooLong,
        LinkError::NotAnAddress,
        LinkError::NotYoutube,
        LinkError::Playlist,
        LinkError::Channel,
        LinkError::NoVideo,
        LinkError::BadVideoId,
    ];
    let mut codes = std::collections::HashSet::new();
    for e in all {
        assert!(e.code().starts_with("youtube_"), "{e:?}");
        assert!(!e.to_string().is_empty());
        codes.insert(e.code());
    }
    // Mehrere Fehler teilen sich eine Meldung (ungueltiger Link), nicht alle.
    assert!(codes.len() >= 6);
}

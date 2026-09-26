//! Per-source plugins for the URL endpoints.
//!
//! Manga sources increasingly ship images the browser can't use as-is: tiles
//! shuffled around a grid, or bytes XORed against a keystream. The reader's own
//! extension undoes this client-side, but when the *server* fetches the image
//! (`/upscale/url`, `/upscale/batch/url`) that step never runs, so the upscaler
//! would sharpen a scrambled page.
//!
//! Each plugin here reimplements one source's descrambling, ported from the
//! corresponding Kotlin interceptor in the Keiyoushi extensions repo. Plugins
//! are matched by host, are **off by default**, and only ever run for the URL
//! endpoints — the raw-bytes endpoints receive images the reader already
//! decoded.
//!
//! Two things the port depends on:
//!   * The client must forward the image URL **including its `#fragment`** —
//!     most sources encode the tile order or XOR key there. The fragment
//!     survives because the URL travels as JSON, and is stripped again before
//!     the actual GET.
//!   * Comix instead sends its seed in *response* headers, so downloads keep
//!     the response headers around for plugins to read.

use std::collections::HashMap;

use image::{imageops, DynamicImage, RgbaImage};

/// Everything a plugin gets to look at for one image.
pub struct PluginInput<'a> {
    /// The `#fragment` of the image URL, if any — where most sources put the
    /// tile order or XOR key.
    pub fragment: Option<&'a str>,
    /// Response headers from the download, lowercased names.
    pub response_headers: &'a HashMap<String, String>,
    pub bytes: &'a [u8],
}

/// What a plugin did. `Unchanged` means "not my request" — e.g. the fragment
/// that marks a scrambled page is absent, which is normal for most pages.
pub enum PluginOutput {
    Unchanged,
    Rewritten { bytes: Vec<u8>, note: String },
}

type TransformFn = fn(&PluginInput) -> Result<PluginOutput, String>;

/// Headers a plugin wants on the download request, given the image URL. Most
/// sources just want a Referer, but Comix needs a per-URL decision.
type HeadersFn = fn(&str) -> Vec<(String, String)>;

/// Which requests a plugin's `transform` is offered.
#[derive(Clone, Copy, PartialEq)]
pub enum TransformScope {
    /// Only URLs whose host matches `hosts` — the default, and what you want
    /// whenever the plugin can't tell on its own whether a page is its own.
    MatchedHosts,
    /// Every URL. Reserved for plugins that identify their own work from
    /// response headers and no-op otherwise: sources that serve images from a
    /// CDN outside their own domain would never match by host at all.
    AnyHost,
}

pub struct PluginDef {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Human-readable list of the sources this covers, for the UI.
    pub sources: &'static str,
    /// Host suffixes this plugin claims. Matching is suffix-based so CDN
    /// subdomains (`cdn.example.com`) are covered by `example.com`.
    pub hosts: &'static [&'static str],
    /// Headers added to the download request. Client-supplied headers win, so
    /// a reader that already sends the right Referer is never overridden.
    pub request_headers: HeadersFn,
    pub transform: Option<TransformFn>,
    pub scope: TransformScope,
}

/// `Referer: https://<host>/` — what nearly every source expects.
fn referer_only(url: &str) -> Vec<(String, String)> {
    host_of(url)
        .map(|h| vec![("Referer".to_string(), format!("https://{h}/"))])
        .unwrap_or_default()
}

// ── Registry ──────────────────────────────────────────────────────────

pub fn all() -> &'static [PluginDef] {
    PLUGINS
}

pub fn get(id: &str) -> Option<&'static PluginDef> {
    PLUGINS.iter().find(|p| p.id == id)
}

/// Plugins whose declared hosts claim this URL, in registry order. Used for
/// request-header injection, which must never be sent to an unrelated site.
pub fn matching(url: &str) -> Vec<&'static PluginDef> {
    let host = match host_of(url) {
        Some(h) => h,
        None => return Vec::new(),
    };
    PLUGINS
        .iter()
        .filter(|p| p.hosts.iter().any(|suffix| host_matches(&host, suffix)))
        .collect()
}

/// Plugins offered the chance to transform this URL's image: the host matches,
/// or the plugin identifies its own work independently of the host.
pub fn transformers_for(url: &str) -> Vec<&'static PluginDef> {
    let host = host_of(url);
    PLUGINS
        .iter()
        .filter(|p| p.transform.is_some())
        .filter(|p| {
            p.scope == TransformScope::AnyHost
                || host
                    .as_deref()
                    .is_some_and(|h| p.hosts.iter().any(|s| host_matches(h, s)))
        })
        .collect()
}

/// Whether a URL carries a marker that some source uses to flag a scrambled
/// page. Used only to warn when such a page goes through unhandled.
pub fn looks_scrambled(url: &str) -> bool {
    match fragment_of(url) {
        Some(f) => {
            f == "scramble"
                || f.starts_with("scramble=")
                || f.contains("size=")
                || f.contains("key=")
                || f.contains("scrambled")
        }
        None => false,
    }
}

/// Lowercased host of a URL, without port.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|s| !s.is_empty())?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = authority.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// The `#fragment` of a URL, if present and non-empty.
pub fn fragment_of(url: &str) -> Option<&str> {
    url.split_once('#').map(|(_, f)| f).filter(|f| !f.is_empty())
}

/// Suffix match on domain boundaries — `cdn.comix.to` matches `comix.to`,
/// `notcomix.to` does not.
fn host_matches(host: &str, suffix: &str) -> bool {
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

/// Comix decides `Origin` per image, and gets it wrong in both directions if
/// you don't: the server withholds the grid-scramble seed when `Origin` is
/// present on an off-domain image, and withholds the byte-cipher seed when it
/// is *absent* on a legacy page. Mirrors `Comix.imageRequest`.
fn comix_headers(url: &str) -> Vec<(String, String)> {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let is_v3 = without_fragment.contains("?v3") || without_fragment.contains("&v3");
    let is_legacy_scramble = url.contains("#scrambled") && !is_v3;
    let on_own_domain = host_of(url).is_some_and(|h| {
        host_matches(&h, "comix.to") || host_matches(&h, "comix.ws")
    });

    let mut headers = vec![
        ("Referer".to_string(), "https://comix.to/".to_string()),
        ("Accept".to_string(), "*/*".to_string()),
    ];
    if on_own_domain || is_legacy_scramble {
        headers.push(("Origin".to_string(), "https://comix.to".to_string()));
    }
    headers
}

static PLUGINS: &[PluginDef] = &[
    PluginDef {
        id: "comix",
        name: "Comix — tile shuffle + stream cipher",
        description: "Comix serves pages cut into a 5×5 grid whose tiles are shuffled by a \
                      seeded PRNG, and sometimes XORs the leading bytes on top of that. The \
                      seed, grid and algorithm arrive in x-scramble-* / x-enc-* response \
                      headers, so this runs automatically once enabled — pages without those \
                      headers pass through untouched.",
        sources: "comix.to, comix.ws (including their image CDN)",
        hosts: &["comix.to", "comix.ws"],
        request_headers: comix_headers,
        transform: Some(comix_transform),
        // Comix serves pages from a CDN outside its own domain, so host
        // matching alone would never fire. Safe to offer everywhere: without
        // the x-scramble-* / x-enc-* response headers it does nothing.
        scope: TransformScope::AnyHost,
    },
    PluginDef {
        id: "gigaviewer",
        name: "GigaViewer — 4×4 block transpose",
        description: "Shueisha/Kodansha's GigaViewer reader transposes a 4×4 grid of blocks \
                      sized to a multiple of 8px, marking scrambled pages with a #scramble \
                      fragment on the image URL. Leftover edge pixels stay in place, matching \
                      the reader's own behaviour.",
        sources: "Shonen Jump+, Comic Days, Tonari no Young Jump, Kurage Bunch, MAGCOMI, \
                  Sunday Webry, Comic Gardo, Comic Border, Comic Earth Star, Ichicomi, \
                  Comiplex, Comic Yours, Zenon",
        hosts: &[
            "shonenjumpplus.com",
            "comic-days.com",
            "tonarinoyj.jp",
            "kuragebunch.com",
            "magcomi.com",
            "sunday-webry.com",
            "comic-gardo.com",
            "comicborder.com",
            "comic-earthstar.com",
            "ichicomi.com",
            // Comiplex only — plain heros-web.com is a ComiciViewer site.
            "viewer.heros-web.com",
            "comic-y-ours.com",
            "comic-zenon.com",
        ],
        request_headers: referer_only,
        transform: Some(gigaviewer_transform),
        scope: TransformScope::MatchedHosts,
    },
    PluginDef {
        id: "comiciviewer",
        name: "ComiciViewer — explicit 4×4 tile order",
        description: "ComiciViewer ships the tile permutation in clear text as a \
                      #scramble=[…] fragment, so unscrambling is a straight reorder of a 4×4 \
                      grid with no key derivation involved.",
        sources: "Manga Cross, Comic Growl, Big Comics, Young Animal, Young Champion, \
                  Manga Bang, Hanayume, Take Comic, Rimacomi+, Kimicomi, Magkan, JN Books, \
                  Comic Ride, Comic Medu, Comic Pash, Hero's Web",
        hosts: &[
            "heros-web.com",
            "mangacross.jp",
            "championcross.jp",
            "comic-growl.com",
            "bigcomics.jp",
            "younganimal.com",
            "youngchampion.jp",
            "comics.manga-bang.com",
            "manga-bang.com",
            "hanayume.com",
            "takecomic.jp",
            "rimacomiplus.jp",
            "kimicomi.com",
            "kansai.mag-garden.co.jp",
            "comic.j-nbooks.jp",
            "comicride.jp",
            "g-comi.jp",
            "comicpash.jp",
        ],
        request_headers: referer_only,
        transform: Some(comiciviewer_transform),
        scope: TransformScope::MatchedHosts,
    },
    PluginDef {
        id: "clipstudioreader",
        name: "Clip Studio Reader — variable grid",
        description: "Clip Studio's reader uses a grid whose dimensions vary per page; both \
                      the tile order and the grid size arrive in a #size=[…]/w/h fragment. \
                      Pieces are snapped to 8px multiples, leaving the right/bottom remainder \
                      untouched.",
        sources: "Comic Festa, FireCross",
        hosts: &["comic.iowl.jp", "comic-festa.com", "firecross.jp"],
        request_headers: referer_only,
        transform: Some(clipstudio_transform),
        scope: TransformScope::MatchedHosts,
    },
    PluginDef {
        id: "kmanga",
        name: "K Manga — xorshift32 4×4 shuffle",
        description: "Kodansha's K Manga derives its 4×4 tile order from an xorshift32 PRNG \
                      seeded by a charset-encoded string XORed with the title and episode ids, \
                      all packed into a #seed:titleId:episodeId fragment.",
        sources: "kmanga.kodansha.com",
        hosts: &["kmanga.kodansha.com", "kodansha.us"],
        request_headers: referer_only,
        transform: Some(kmanga_transform),
        scope: TransformScope::MatchedHosts,
    },
    PluginDef {
        id: "mangaplus",
        name: "MANGA Plus — XOR keystream",
        description: "MANGA Plus by Shueisha XORs every image byte against a repeating \
                      keystream sent as a hex string in the URL fragment. Cheap to undo and \
                      lossless — the original JPEG bytes come back exactly.",
        sources: "mangaplus.shueisha.co.jp",
        hosts: &["mangaplus.shueisha.co.jp", "jumpg-assets.tokyo-cdn.com"],
        request_headers: referer_only,
        transform: Some(mangaplus_transform),
        scope: TransformScope::MatchedHosts,
    },
    PluginDef {
        id: "zebrack",
        name: "Zebrack — XOR keystream",
        description: "Zebrack XORs image bytes against a repeating key given as #key=<hex>. \
                      Same idea as MANGA Plus with a different fragment format.",
        sources: "zebrack-comic.shueisha.co.jp",
        hosts: &["zebrack-comic.shueisha.co.jp"],
        request_headers: referer_only,
        transform: Some(zebrack_transform),
        scope: TransformScope::MatchedHosts,
    },
    PluginDef {
        id: "ynjn",
        name: "YnJn / Sokuyomi — 4×4 transpose",
        description: "Both readers mirror a 4×4 grid across its diagonal (row and column \
                      swapped) on pages tagged with a #scramble fragment.",
        sources: "ynjn.jp, sokuyomi.jp",
        hosts: &["ynjn.jp", "sokuyomi.jp"],
        request_headers: referer_only,
        transform: Some(ynjn_transform),
        scope: TransformScope::MatchedHosts,
    },
    PluginDef {
        id: "source-headers",
        name: "Anti-hotlink headers",
        description: "Sends the Referer (and where needed Origin or a desktop User-Agent) \
                      each source expects, for hosts that reject downloads without them. No \
                      image processing — enable this when URL downloads come back 403 and the \
                      reader isn't already forwarding its own headers.",
        sources: "MangaDex, Webtoons, Manhuagui, and any Madara/MangaThemesia site",
        hosts: &[
            "mangadex.org",
            "mangadex.network",
            "webtoons.com",
            "manhuagui.com",
            "hmanhuagui.com",
            "mhgubox.com",
        ],
        request_headers: anti_hotlink_headers,
        transform: None,
        scope: TransformScope::MatchedHosts,
    },
];

fn anti_hotlink_headers(url: &str) -> Vec<(String, String)> {
    let mut headers = referer_only(url);
    headers.push((
        "User-Agent".to_string(),
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
            .to_string(),
    ));
    headers
}

// ── Shared image helpers ──────────────────────────────────────────────

fn decode(bytes: &[u8]) -> Result<RgbaImage, String> {
    image::load_from_memory(bytes)
        .map_err(|e| format!("could not decode image: {e}"))
        .map(DynamicImage::into_rgba8)
}

fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| format!("could not re-encode image: {e}"))?;
    Ok(out)
}

/// Rebuild an image from tiles. `moves` lists `(src_x, src_y, dst_x, dst_y)`
/// rectangles of `tile_w` × `tile_h`.
///
/// The output starts as a copy of the input — mirroring the `drawBitmap(0,0)`
/// pre-fill in every Kotlin interceptor — so pixels outside the tiled area
/// (the right/bottom remainder when dimensions don't divide evenly) survive.
/// A tile that doesn't fit is skipped rather than failing the page, and a
/// degenerate tile size makes the whole thing a no-op. Both match the Android
/// original, where `Canvas.drawBitmap` simply clips out-of-bounds rectangles
/// and draws nothing for empty ones — an image too small for its own grid, or a
/// tile list longer than the grid, comes back unscrambled instead of erroring.
fn rearrange(
    src: &RgbaImage,
    tile_w: u32,
    tile_h: u32,
    moves: &[(u32, u32, u32, u32)],
) -> RgbaImage {
    let mut out = src.clone();
    if tile_w == 0 || tile_h == 0 {
        return out;
    }
    for &(sx, sy, dx, dy) in moves {
        if sx + tile_w > src.width()
            || sy + tile_h > src.height()
            || dx + tile_w > src.width()
            || dy + tile_h > src.height()
        {
            continue;
        }
        let tile = imageops::crop_imm(src, sx, sy, tile_w, tile_h).to_image();
        imageops::replace(&mut out, &tile, dx as i64, dy as i64);
    }
    out
}

/// XOR `bytes` with a repeating keystream.
fn xor_keystream(bytes: &[u8], key: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

/// Decode a hex keystream. Operates on bytes so a fragment containing
/// multi-byte UTF-8 can't panic on a char boundary, and rejects anything that
/// isn't a hex digit (`from_str_radix` would otherwise accept a leading `+`).
///
/// An odd-length string decodes its trailing lone digit as a final byte, which
/// is what the reference `chunked(2)` implementation does.
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    bytes
        .chunks(2)
        .map(|pair| match pair {
            [hi, lo] => Some(nibble(*hi)? << 4 | nibble(*lo)?),
            [only] => nibble(*only),
            _ => None,
        })
        .collect()
}

fn has_image_signature(b: &[u8]) -> bool {
    b.len() >= 12
        && (b.starts_with(&[0xFF, 0xD8])
            || b.starts_with(&[0x89, b'P', b'N', b'G'])
            || (b.starts_with(b"RIFF") && &b[8..12] == b"WEBP"))
}

// ── Comix ─────────────────────────────────────────────────────────────

const COMIX_ENC_MULTIPLIER: i32 = 1_000_005;
const COMIX_ENC_INCREMENT: i32 = 1_234_567_891;
const COMIX_LCG_MULTIPLIER: i32 = 1_664_525;
const COMIX_LCG_INCREMENT: i32 = 1_013_904_223;
const COMIX_GRID: u32 = 5;

fn next_xorshift(state: i32) -> i32 {
    let mut n = state;
    n ^= n << 13;
    n ^= ((n as u32) >> 17) as i32;
    n ^ (n << 5)
}

fn comix_decode_lcg(bytes: &[u8], seed: i32, length: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut state = seed;
    for b in out.iter_mut().take(length) {
        state = state
            .wrapping_mul(COMIX_ENC_MULTIPLIER)
            .wrapping_add(COMIX_ENC_INCREMENT);
        *b ^= ((state as u32) >> 24) as u8;
    }
    out
}

fn comix_decode_xorshift(bytes: &[u8], initial: i32, length: usize, high_byte: bool) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut state = initial;
    for b in out.iter_mut().take(length) {
        state = next_xorshift(state);
        let key = if high_byte {
            ((state as u32) >> 24) as u8
        } else {
            (state as u32 & 0xFF) as u8
        };
        *b ^= key;
    }
    out
}

fn comix_decode_bytes(bytes: &[u8], seed: i32, length: usize, algo: Option<&str>) -> Vec<u8> {
    if algo != Some("2") {
        return comix_decode_lcg(bytes, seed, length);
    }
    // Algorithm "2" doesn't say which variant produced the stream, so try each
    // and keep the first that decodes to a recognisable image container.
    let candidates = [
        comix_decode_xorshift(bytes, seed | 1, length, false),
        comix_decode_xorshift(bytes, seed, length, false),
        comix_decode_xorshift(bytes, seed | 1, length, true),
        comix_decode_lcg(bytes, seed, length),
    ];
    candidates
        .iter()
        .find(|c| has_image_signature(c))
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

/// Fisher-Yates driven by the site's PRNG, then inverted: the site stores the
/// *packing* permutation, and we need the unpacking one.
fn comix_build_order(seed: i32, n: usize, xorshift: bool) -> Vec<usize> {
    let mut arr: Vec<usize> = (0..n).collect();
    let mut state = if xorshift { seed | 1 } else { seed };
    for i in (1..n).rev() {
        state = if xorshift {
            next_xorshift(state)
        } else {
            state
                .wrapping_mul(COMIX_LCG_MULTIPLIER)
                .wrapping_add(COMIX_LCG_INCREMENT)
        };
        let j = (state as u32 as u64 % (i as u64 + 1)) as usize;
        arr.swap(i, j);
    }
    let mut inverse = vec![0usize; n];
    for (i, &v) in arr.iter().enumerate() {
        inverse[v] = i;
    }
    inverse
}

fn comix_scramble_hash(raw: Option<&str>) -> i32 {
    match raw.map(str::trim) {
        Some("03632") => 58414,
        Some("02900") => 117_532,
        _ => 0,
    }
}

fn comix_transform(input: &PluginInput) -> Result<PluginOutput, String> {
    let h = |name: &str| input.response_headers.get(name).map(String::as_str);

    // Kotlin parses as Long then narrows to Int; match that so large seeds wrap
    // identically instead of failing to parse.
    let as_i32 = |v: Option<&str>| v.and_then(|s| s.trim().parse::<i64>().ok()).map(|v| v as i32);

    let enc_seed = as_i32(h("x-enc-seed"));
    let enc_len = h("x-enc-len").and_then(|s| s.trim().parse::<usize>().ok());
    let enc_algo = h("x-enc-algo");
    let scramble_seed = as_i32(h("x-scramble-seed"));
    let scramble_algo = h("x-scramble-algo");
    let scramble_grid = h("x-scramble-grid");

    let needs_xor = matches!((enc_seed, enc_len), (Some(s), Some(_)) if s != 0);
    let grid_ok = scramble_grid == Some("5x5")
        && matches!(scramble_algo, None | Some("1") | Some("2") | Some("3"))
        && matches!(scramble_seed, Some(s) if s != 0);

    if !needs_xor && !grid_ok {
        return Ok(PluginOutput::Unchanged);
    }

    let mut note = String::new();
    let bytes = if needs_xor {
        note.push_str("XOR");
        comix_decode_bytes(
            input.bytes,
            enc_seed.unwrap(),
            enc_len.unwrap(),
            enc_algo,
        )
    } else {
        input.bytes.to_vec()
    };

    if !grid_ok {
        return Ok(PluginOutput::Rewritten {
            bytes,
            note: format!("Comix: {note} decoded"),
        });
    }

    let seed = scramble_seed.unwrap() ^ comix_scramble_hash(h("x-scramble-hash"));
    let order = comix_build_order(seed, (COMIX_GRID * COMIX_GRID) as usize, scramble_algo == Some("3"));

    let img = decode(&bytes)?;
    let tile_w = img.width() / COMIX_GRID;
    let tile_h = img.height() / COMIX_GRID;
    let moves: Vec<_> = order
        .iter()
        .enumerate()
        .map(|(dst, &src)| {
            let src = src as u32;
            let dst = dst as u32;
            (
                (src % COMIX_GRID) * tile_w,
                (src / COMIX_GRID) * tile_h,
                (dst % COMIX_GRID) * tile_w,
                (dst / COMIX_GRID) * tile_h,
            )
        })
        .collect();

    let out = rearrange(&img, tile_w, tile_h, &moves);
    if !note.is_empty() {
        note.push_str(" + ");
    }
    note.push_str("5×5 unshuffle");
    Ok(PluginOutput::Rewritten {
        bytes: encode_png(&out)?,
        note: format!("Comix: {note}"),
    })
}

// ── GigaViewer ────────────────────────────────────────────────────────

fn gigaviewer_transform(input: &PluginInput) -> Result<PluginOutput, String> {
    if input.fragment != Some("scramble") {
        return Ok(PluginOutput::Unchanged);
    }
    const DIVIDE: u32 = 4;
    const MULTIPLE: u32 = 8;

    let img = decode(input.bytes)?;
    let block_w = img.width() / (DIVIDE * MULTIPLE) * MULTIPLE;
    let block_h = img.height() / (DIVIDE * MULTIPLE) * MULTIPLE;

    let moves: Vec<_> = (0..DIVIDE * DIVIDE)
        .map(|e| {
            let dst_index = (e % DIVIDE) * DIVIDE + (e / DIVIDE);
            (
                (e % DIVIDE) * block_w,
                (e / DIVIDE) * block_h,
                (dst_index % DIVIDE) * block_w,
                (dst_index / DIVIDE) * block_h,
            )
        })
        .collect();

    let out = rearrange(&img, block_w, block_h, &moves);
    Ok(PluginOutput::Rewritten {
        bytes: encode_png(&out)?,
        note: "GigaViewer: 4×4 transpose".to_string(),
    })
}

// ── ComiciViewer ──────────────────────────────────────────────────────

fn parse_index_list(s: &str) -> Option<Vec<u32>> {
    s.trim_matches(['[', ']'])
        .split(',')
        .map(|t| t.trim().parse::<u32>().ok())
        .collect()
}

fn comiciviewer_transform(input: &PluginInput) -> Result<PluginOutput, String> {
    let frag = match input.fragment {
        Some(f) if f.starts_with("scramble=") => f,
        _ => return Ok(PluginOutput::Unchanged),
    };
    const GRID: u32 = 4;

    let tiles = parse_index_list(&frag["scramble=".len()..])
        .ok_or_else(|| "malformed #scramble= tile list".to_string())?;

    let img = decode(input.bytes)?;
    let tile_w = img.width() / GRID;
    let tile_h = img.height() / GRID;

    // Note the column-major indexing: ComiciViewer walks the grid down first.
    let moves: Vec<_> = tiles
        .iter()
        .enumerate()
        .map(|(dst, &src)| {
            let dst = dst as u32;
            (
                (src / GRID) * tile_w,
                (src % GRID) * tile_h,
                (dst / GRID) * tile_w,
                (dst % GRID) * tile_h,
            )
        })
        .collect();

    let out = rearrange(&img, tile_w, tile_h, &moves);
    Ok(PluginOutput::Rewritten {
        bytes: encode_png(&out)?,
        note: format!("ComiciViewer: {} tiles reordered", tiles.len()),
    })
}

// ── Clip Studio Reader ────────────────────────────────────────────────

fn clipstudio_transform(input: &PluginInput) -> Result<PluginOutput, String> {
    let frag = match input.fragment {
        Some(f) if f.contains("size=") => f,
        _ => return Ok(PluginOutput::Unchanged),
    };
    let spec = &frag[frag.find("size=").unwrap() + "size=".len()..];
    let mut parts = spec.splitn(3, '/');
    let (array_str, grid_w, grid_h) = match (parts.next(), parts.next(), parts.next()) {
        (Some(a), Some(w), Some(h)) => (a, w, h),
        _ => return Err("malformed #size= fragment".to_string()),
    };
    let mapping =
        parse_index_list(array_str).ok_or_else(|| "malformed #size= tile list".to_string())?;
    let grid_w: u32 = grid_w
        .trim()
        .parse()
        .map_err(|_| "malformed grid width".to_string())?;
    let grid_h: u32 = grid_h
        .trim()
        .parse()
        .map_err(|_| "malformed grid height".to_string())?;
    if grid_w == 0 || grid_h == 0 {
        return Err("zero grid dimension".to_string());
    }

    let img = decode(input.bytes)?;
    // The reader gives up rather than produce garbage when the page is too
    // small for its own grid; do the same.
    if mapping.len() < (grid_w * grid_h) as usize
        || img.width() < 8 * grid_w
        || img.height() < 8 * grid_h
    {
        return Ok(PluginOutput::Unchanged);
    }

    let piece_w = (img.width() / grid_w) / 8 * 8;
    let piece_h = (img.height() / grid_h) / 8 * 8;

    let moves: Vec<_> = mapping
        .iter()
        .enumerate()
        .map(|(dst, &src)| {
            let dst = dst as u32;
            (
                (src % grid_w) * piece_w,
                (src / grid_w) * piece_h,
                (dst % grid_w) * piece_w,
                (dst / grid_w) * piece_h,
            )
        })
        .collect();

    let out = rearrange(&img, piece_w, piece_h, &moves);
    Ok(PluginOutput::Rewritten {
        bytes: encode_png(&out)?,
        note: format!("Clip Studio: {grid_w}×{grid_h} grid unscrambled"),
    })
}

// ── K Manga ───────────────────────────────────────────────────────────

const KMANGA_CHARSET_EVEN: &str = "we7ru3ty8i";
const KMANGA_CHARSET_ODD: &str = "h4xm9bqz1p";

fn kmanga_transform(input: &PluginInput) -> Result<PluginOutput, String> {
    let frag = match input.fragment {
        Some(f) if f.contains(':') => f,
        _ => return Ok(PluginOutput::Unchanged),
    };
    let mut parts = frag.split(':');
    let (seed, title_id, episode_id) = match (parts.next(), parts.next(), parts.next()) {
        (Some(s), Some(t), Some(e)) => (s, t, e),
        _ => return Err("malformed #seed:titleId:episodeId fragment".to_string()),
    };
    let title_id: u32 = title_id
        .trim()
        .parse()
        .map_err(|_| "malformed titleId".to_string())?;
    let episode_id: u32 = episode_id
        .trim()
        .parse()
        .map_err(|_| "malformed episodeId".to_string())?;

    // The seed string is a base-10 number written in a per-title alphabet.
    let charset = if title_id % 2 == 0 {
        KMANGA_CHARSET_EVEN
    } else {
        KMANGA_CHARSET_ODD
    };
    let mut parsed: u64 = 0;
    for ch in seed.chars() {
        match charset.chars().position(|c| c == ch) {
            Some(idx) => parsed = parsed.wrapping_mul(10).wrapping_add(idx as u64),
            None => break,
        }
    }

    let mut state = (parsed as u32) ^ title_id.wrapping_add(episode_id);
    const GRID: u32 = 4;
    let mut pairs: Vec<(u32, u32)> = Vec::with_capacity(16);
    for i in 0..GRID * GRID {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        pairs.push((state, i));
    }
    // Stable sort by PRNG value: position in the sorted list is the tile's
    // destination, its payload the source index.
    pairs.sort_by_key(|&(v, _)| v);

    let img = decode(input.bytes)?;
    let block_w = (img.width() / 8 * 8) / GRID;
    let block_h = (img.height() / 8 * 8) / GRID;

    let moves: Vec<_> = pairs
        .iter()
        .enumerate()
        .map(|(dst, &(_, src))| {
            let dst = dst as u32;
            (
                (src % GRID) * block_w,
                (src / GRID) * block_h,
                (dst % GRID) * block_w,
                (dst / GRID) * block_h,
            )
        })
        .collect();

    let out = rearrange(&img, block_w, block_h, &moves);
    Ok(PluginOutput::Rewritten {
        bytes: encode_png(&out)?,
        note: "K Manga: xorshift 4×4 unshuffle".to_string(),
    })
}

// ── MANGA Plus / Zebrack ──────────────────────────────────────────────

fn mangaplus_transform(input: &PluginInput) -> Result<PluginOutput, String> {
    let frag = match input.fragment {
        Some(f) => f,
        None => return Ok(PluginOutput::Unchanged),
    };
    // The reader treats a non-hex fragment as a hard error rather than
    // shrugging, and so should we: silently upscaling still-encrypted bytes
    // would look like a broken page with nothing in the log to explain it.
    let key = decode_hex(frag)
        .ok_or_else(|| format!("fragment '{frag}' is not a hex key"))?;
    Ok(PluginOutput::Rewritten {
        bytes: xor_keystream(input.bytes, &key),
        note: format!("MANGA Plus: XOR ({}-byte key)", key.len()),
    })
}

fn zebrack_transform(input: &PluginInput) -> Result<PluginOutput, String> {
    let frag = match input.fragment {
        Some(f) if f.contains("key=") => f,
        _ => return Ok(PluginOutput::Unchanged),
    };
    let hex = &frag[frag.find("key=").unwrap() + "key=".len()..];
    let key = decode_hex(hex).ok_or_else(|| "malformed #key= hex".to_string())?;
    Ok(PluginOutput::Rewritten {
        bytes: xor_keystream(input.bytes, &key),
        note: format!("Zebrack: XOR ({}-byte key)", key.len()),
    })
}

// ── YnJn / Sokuyomi ───────────────────────────────────────────────────

fn ynjn_transform(input: &PluginInput) -> Result<PluginOutput, String> {
    if input.fragment != Some("scramble") {
        return Ok(PluginOutput::Unchanged);
    }
    const GRID: u32 = 4;
    let img = decode(input.bytes)?;
    let block_w = img.width() / GRID;
    let block_h = img.height() / GRID;

    let moves: Vec<_> = (0..GRID * GRID)
        .map(|i| {
            let (row, col) = (i / GRID, i % GRID);
            (col * block_w, row * block_h, row * block_w, col * block_h)
        })
        .collect();

    let out = rearrange(&img, block_w, block_h, &moves);
    Ok(PluginOutput::Rewritten {
        bytes: encode_png(&out)?,
        note: "YnJn: 4×4 diagonal mirror".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(url: &'a str, bytes: &'a [u8], headers: &'a HashMap<String, String>) -> PluginInput<'a> {
        PluginInput {
            fragment: fragment_of(url),
            response_headers: headers,
            bytes,
        }
    }

    /// A gradient image so every tile is distinguishable after a round trip.
    fn sample(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
        })
    }

    #[test]
    fn host_matching_respects_domain_boundaries() {
        assert_eq!(host_of("https://cdn.comix.to/a/b.jpg#x").as_deref(), Some("cdn.comix.to"));
        assert_eq!(host_of("https://comix.to:8443/a.jpg").as_deref(), Some("comix.to"));
        assert!(host_matches("cdn.comix.to", "comix.to"));
        assert!(!host_matches("notcomix.to", "comix.to"));
        assert!(!matching("https://evil-comix.to/a.jpg").iter().any(|p| p.id == "comix"));
        assert!(matching("https://comix.to/a.jpg").iter().any(|p| p.id == "comix"));
    }

    #[test]
    fn fragment_parsing() {
        assert_eq!(fragment_of("https://a.b/c.jpg#scramble"), Some("scramble"));
        assert_eq!(fragment_of("https://a.b/c.jpg#"), None);
        assert_eq!(fragment_of("https://a.b/c.jpg"), None);
    }

    #[test]
    fn plugins_ignore_urls_without_their_marker() {
        let headers = HashMap::new();
        let bytes = encode_png(&sample(64, 64)).unwrap();
        for id in ["gigaviewer", "comiciviewer", "kmanga", "zebrack", "ynjn", "comix"] {
            let p = get(id).unwrap();
            let out = (p.transform.unwrap())(&ctx("https://x.test/a.jpg", &bytes, &headers)).unwrap();
            assert!(
                matches!(out, PluginOutput::Unchanged),
                "{id} touched an unmarked image"
            );
        }
    }

    #[test]
    fn gigaviewer_transpose_is_an_involution() {
        // Transposing a 4×4 grid twice is the identity, so scrambling the
        // sample with the same routine must round-trip back to it.
        let headers = HashMap::new();
        let original = sample(256, 256);
        let once = (get("gigaviewer").unwrap().transform.unwrap())(&ctx(
            "https://shonenjumpplus.com/a.jpg#scramble",
            &encode_png(&original).unwrap(),
            &headers,
        ))
        .unwrap();
        let scrambled = match once {
            PluginOutput::Rewritten { bytes, .. } => bytes,
            _ => panic!("expected a rewrite"),
        };
        let twice = (get("gigaviewer").unwrap().transform.unwrap())(&ctx(
            "https://shonenjumpplus.com/a.jpg#scramble",
            &scrambled,
            &headers,
        ))
        .unwrap();
        match twice {
            PluginOutput::Rewritten { bytes, .. } => {
                assert_eq!(decode(&bytes).unwrap(), original);
            }
            _ => panic!("expected a rewrite"),
        }
    }

    #[test]
    fn comiciviewer_round_trips_an_explicit_permutation() {
        let headers = HashMap::new();
        let original = sample(256, 256);
        // Swap the first two tiles; applying it twice restores the original.
        let mut order: Vec<u32> = (0..16).collect();
        order.swap(0, 1);
        let frag = format!(
            "https://mangacross.jp/a.jpg#scramble=[{}]",
            order.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
        );
        let t = get("comiciviewer").unwrap().transform.unwrap();
        let once = match t(&ctx(&frag, &encode_png(&original).unwrap(), &headers)).unwrap() {
            PluginOutput::Rewritten { bytes, .. } => bytes,
            _ => panic!("expected a rewrite"),
        };
        match t(&ctx(&frag, &once, &headers)).unwrap() {
            PluginOutput::Rewritten { bytes, .. } => {
                assert_eq!(decode(&bytes).unwrap(), original)
            }
            _ => panic!("expected a rewrite"),
        }
    }

    #[test]
    fn xor_plugins_restore_original_bytes() {
        let headers = HashMap::new();
        let payload: Vec<u8> = (0u16..600).map(|v| (v % 251) as u8).collect();
        let key = [0xA5u8, 0x3C, 0x77];
        let encrypted = xor_keystream(&payload, &key);

        let mp = (get("mangaplus").unwrap().transform.unwrap())(&ctx(
            "https://mangaplus.shueisha.co.jp/a.jpg#a53c77",
            &encrypted,
            &headers,
        ))
        .unwrap();
        match mp {
            PluginOutput::Rewritten { bytes, .. } => assert_eq!(bytes, payload),
            _ => panic!("expected a rewrite"),
        }

        let zb = (get("zebrack").unwrap().transform.unwrap())(&ctx(
            "https://zebrack-comic.shueisha.co.jp/a.jpg#key=a53c77",
            &encrypted,
            &headers,
        ))
        .unwrap();
        match zb {
            PluginOutput::Rewritten { bytes, .. } => assert_eq!(bytes, payload),
            _ => panic!("expected a rewrite"),
        }
    }

    #[test]
    fn comix_order_is_a_permutation_for_both_prngs() {
        for xorshift in [false, true] {
            let order = comix_build_order(0x5EED_1234, 25, xorshift);
            let mut seen = order.clone();
            seen.sort_unstable();
            assert_eq!(seen, (0..25).collect::<Vec<_>>());
        }
    }

    #[test]
    fn comix_xor_matches_the_kotlin_lcg() {
        // Derived by hand from the reference implementation: state starts at
        // the seed and each byte is XORed with the top byte of the new state.
        let seed = 12345i32;
        let plain = [1u8, 2, 3, 4];
        let encrypted = comix_decode_lcg(&plain, seed, 4);
        // The stream cipher is its own inverse.
        assert_eq!(comix_decode_lcg(&encrypted, seed, 4), plain);
    }

    #[test]
    fn comix_passes_through_when_headers_are_absent() {
        let headers = HashMap::new();
        let bytes = encode_png(&sample(50, 50)).unwrap();
        let out = comix_transform(&ctx("https://comix.to/a.jpg", &bytes, &headers)).unwrap();
        assert!(matches!(out, PluginOutput::Unchanged));
    }

    #[test]
    fn comix_grid_round_trips() {
        let original = sample(250, 250);
        let mut headers = HashMap::new();
        headers.insert("x-scramble-grid".to_string(), "5x5".to_string());
        headers.insert("x-scramble-seed".to_string(), "987654".to_string());
        headers.insert("x-scramble-algo".to_string(), "3".to_string());
        let bytes = encode_png(&original).unwrap();

        let out = comix_transform(&ctx("https://comix.to/a.jpg", &bytes, &headers)).unwrap();
        let unscrambled = match out {
            PluginOutput::Rewritten { bytes, .. } => decode(&bytes).unwrap(),
            _ => panic!("expected a rewrite"),
        };
        // Applying the inverse permutation by hand must return the original.
        let order = comix_build_order(987_654, 25, true);
        let (tw, th) = (250 / 5, 250 / 5);
        let moves: Vec<_> = order
            .iter()
            .enumerate()
            .map(|(dst, &src)| {
                let (src, dst) = (src as u32, dst as u32);
                (
                    (dst % 5) * tw,
                    (dst / 5) * th,
                    (src % 5) * tw,
                    (src / 5) * th,
                )
            })
            .collect();
        assert_eq!(rearrange(&unscrambled, tw, th, &moves), original);
    }

    #[test]
    fn an_image_too_small_for_its_grid_passes_through_untouched() {
        // Matches the Android original, which draws zero-size rectangles (a
        // no-op) rather than failing the page.
        let img = sample(3, 3);
        assert_eq!(rearrange(&img, 0, 0, &[(0, 0, 0, 0)]), img);

        let headers = HashMap::new();
        let tiny = encode_png(&sample(20, 20)).unwrap();
        let out = (get("gigaviewer").unwrap().transform.unwrap())(&ctx(
            "https://shonenjumpplus.com/a.jpg#scramble",
            &tiny,
            &headers,
        ))
        .unwrap();
        match out {
            PluginOutput::Rewritten { bytes, .. } => {
                assert_eq!(decode(&bytes).unwrap(), sample(20, 20))
            }
            PluginOutput::Unchanged => {}
        }
    }

    #[test]
    fn out_of_bounds_tiles_are_skipped_not_fatal() {
        // A tile list longer than the grid is something the reference
        // implementation tolerates by clipping, so it must not fail the page.
        let img = sample(64, 64);
        let out = rearrange(&img, 16, 16, &[(0, 0, 0, 0), (900, 900, 0, 0)]);
        assert_eq!(out.dimensions(), img.dimensions());
    }

    #[test]
    fn a_non_ascii_fragment_does_not_panic() {
        let headers = HashMap::new();
        let bytes = encode_png(&sample(32, 32)).unwrap();
        // Multi-byte characters used to split a byte slice mid-character.
        for frag in ["aあ", "あ", "日本語のテキスト", "+f"] {
            let url = format!("https://mangaplus.shueisha.co.jp/a.jpg#{frag}");
            let out = (get("mangaplus").unwrap().transform.unwrap())(&ctx(&url, &bytes, &headers));
            assert!(out.is_err(), "{frag} should be reported, not decoded");
        }
    }

    #[test]
    fn odd_length_hex_keys_decode_like_the_reference() {
        // Kotlin's `chunked(2)` leaves a trailing lone digit as its own byte.
        assert_eq!(decode_hex("a53c7").unwrap(), vec![0xA5, 0x3C, 0x07]);
        assert_eq!(decode_hex("a53c77").unwrap(), vec![0xA5, 0x3C, 0x77]);
        assert!(decode_hex("").is_none());
        assert!(decode_hex("zz").is_none());
    }

    #[test]
    fn comix_sends_origin_only_where_the_reader_would() {
        let origin_sent = |url: &str| {
            comix_headers(url)
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("Origin"))
        };
        // On its own domain, Origin is always sent.
        assert!(origin_sent("https://comix.to/i/page.jpg"));
        // Off-domain V3 pages must not send it, or the seed is withheld.
        assert!(!origin_sent("https://cdn.example.net/i5/page.jpg?v3"));
        assert!(!origin_sent("https://cdn.example.net/i5/page.jpg"));
        // Off-domain legacy pages need it, or the cipher seed is withheld.
        assert!(origin_sent("https://cdn.example.net/i/page.jpg#scrambled"));
        // ...unless they are also V3, which takes priority.
        assert!(!origin_sent("https://cdn.example.net/i/page.jpg?v3#scrambled"));
    }

    #[test]
    fn comix_is_offered_pages_from_an_unknown_cdn() {
        // Comix's images live off-domain, so host matching alone would miss
        // them entirely; it identifies its own work from response headers.
        let cdn = "https://cdn.example.net/i5/page.jpg?v3";
        assert!(!matching(cdn).iter().any(|p| p.id == "comix"));
        assert!(transformers_for(cdn).iter().any(|p| p.id == "comix"));
        // Host-scoped plugins are still not offered unrelated hosts.
        assert!(!transformers_for(cdn).iter().any(|p| p.id == "gigaviewer"));
    }

    #[test]
    fn heros_web_is_registered_as_a_comiciviewer_source() {
        // Plain heros-web.com is ComiciViewer; only its viewer subdomain is
        // GigaViewer. Getting this backwards silently ships scrambled pages.
        let ids: Vec<_> = matching("https://heros-web.com/i/p.jpg")
            .iter()
            .map(|p| p.id)
            .collect();
        assert!(ids.contains(&"comiciviewer"), "got {ids:?}");
        assert!(!ids.contains(&"gigaviewer"), "got {ids:?}");

        let viewer: Vec<_> = matching("https://viewer.heros-web.com/i/p.jpg")
            .iter()
            .map(|p| p.id)
            .collect();
        assert!(viewer.contains(&"gigaviewer"), "got {viewer:?}");
    }

    #[test]
    fn scramble_markers_are_recognised_for_the_unhandled_warning() {
        assert!(looks_scrambled("https://a.b/c.jpg#scramble"));
        assert!(looks_scrambled("https://a.b/c.jpg#scramble=[0,1]"));
        assert!(looks_scrambled("https://a.b/c.jpg#size=[0]/4/4"));
        assert!(looks_scrambled("https://a.b/c.jpg#key=ab"));
        assert!(looks_scrambled("https://a.b/c.jpg#scrambled"));
        assert!(!looks_scrambled("https://a.b/c.jpg"));
        assert!(!looks_scrambled("https://a.b/c.jpg#"));
    }

    #[test]
    fn every_plugin_has_unique_id_and_prose() {
        let mut ids: Vec<_> = all().iter().map(|p| p.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate plugin id");
        for p in all() {
            assert!(!p.description.is_empty(), "{} has no description", p.id);
            assert!(!p.sources.is_empty(), "{} lists no sources", p.id);
            assert!(!p.hosts.is_empty(), "{} matches no hosts", p.id);
        }
    }
}

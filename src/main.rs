//! Omarchy screensaver: the koi pond from dao-ho's portfolio, drawn in a terminal.
//!
//! Runs fullscreen in whichever terminal Omarchy's screensaver launcher opened, and exits on a
//! keypress, on a signal, or when its window is no longer focused.

use std::io::{self, IsTerminal, Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use koi_screensaver::render::PondRenderer;
use koi_screensaver::term::Terminal;
use koi_screensaver::world::World;

mod native;

const STEP: f64 = 1.0 / 60.0;
const MAX_STEPS: usize = 4;

static QUIT: AtomicBool = AtomicBool::new(false);
static WINCH: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Default)]
struct WinSize {
    rows: u16,
    cols: u16,
    xpixel: u16,
    ypixel: u16,
}

extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
}

const TIOCGWINSZ: u64 = 0x5413;
const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGQUIT: i32 = 3;
const SIGTERM: i32 = 15;
const SIGWINCH: i32 = 28;

extern "C" fn on_winch(_: i32) {
    WINCH.store(true, Ordering::SeqCst);
}

extern "C" fn on_quit(_: i32) {
    QUIT.store(true, Ordering::SeqCst);
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(default)
}

fn tty_size() -> Option<(usize, usize)> {
    let mut ws = WinSize::default();
    let rc = unsafe { ioctl(1, TIOCGWINSZ, &mut ws as *mut WinSize) };
    if rc == 0 && ws.cols > 0 && ws.rows > 0 {
        Some((ws.cols as usize, ws.rows as usize))
    } else {
        None
    }
}

fn size() -> (usize, usize) {
    if let Ok(forced) = std::env::var("KOI_SIZE") {
        let parts: Vec<usize> = forced
            .split(['x', ','])
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if parts.len() == 2 && parts[0] > 0 && parts[1] > 0 {
            return (parts[0], parts[1]);
        }
    }
    tty_size().unwrap_or_else(|| {
        let cols = std::env::var("COLUMNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80);
        let rows = std::env::var("LINES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
        (cols, rows)
    })
}

/// The pty is 80x24 until the compositor has told the terminal how big the window is, and foot
/// only reports the real grid a moment after that. Seeding the pond at the default size would give
/// three tiny koi on a fullscreen display, so wait for the size to stop moving first.
fn settled_size() -> (usize, usize) {
    let start = Instant::now();
    let deadline = Duration::from_millis(2500);
    let mut last = size();
    let mut stable = 0;
    loop {
        std::thread::sleep(Duration::from_millis(150));
        let now = size();
        if now == last {
            stable += 1;
        } else {
            last = now;
            stable = 0;
        }
        let elapsed = start.elapsed();
        if stable >= 2 && (now != (80, 24) || elapsed >= Duration::from_millis(1500)) {
            break;
        }
        if elapsed >= deadline {
            break;
        }
    }
    last
}

fn stty(args: &[&str]) -> Option<String> {
    let out = Command::new("stty")
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

struct WindowInfo {
    class: String,
    width: f64,
    height: f64,
}

/// The focused window, from Hyprland: its class (so we can quit when focus moves) and its size in
/// layout pixels, which together with the pty size gives the real shape of a terminal cell.
fn window_info() -> Option<WindowInfo> {
    let out = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);

    let class_key = "\"class\":";
    let start = text.find(class_key)? + class_key.len();
    let rest = &text[start..];
    let open = rest.find('"')? + 1;
    let close = rest[open..].find('"')? + open;
    let class = rest[open..close].to_string();

    let size_key = "\"size\"";
    let start = text.find(size_key)? + size_key.len();
    let rest = &text[start..];
    let open = rest.find('[')? + 1;
    let close = rest[open..].find(']')? + open;
    let parts: Vec<f64> = rest[open..close]
        .split(',')
        .filter_map(|p| p.trim().parse::<f64>().ok())
        .collect();
    let (width, height) = match parts.as_slice() {
        [w, h] => (*w, *h),
        _ => (0.0, 0.0),
    };
    Some(WindowInfo {
        class,
        width,
        height,
    })
}

fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil conversion, so the stats log needs no date library.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Follow a terminal resize. A pond whose area changes a long way (the window finishing its
/// fullscreen animation, or a different monitor) is reseeded at the new size instead of keeping the
/// few small koi that were placed for the old one.
fn apply_size(
    renderer: &mut PondRenderer,
    terminal: &mut Terminal,
    world: &mut World,
    cols: usize,
    rows: usize,
) {
    let before = renderer.width() * renderer.height();
    renderer.resize(cols, rows);
    terminal.resize(cols, rows);
    world.resize(renderer.width(), renderer.height());
    let after = renderer.width() * renderer.height();
    let ratio = if before > 0.0 { after / before } else { 1.0 };
    if !(0.8..=1.25).contains(&ratio) {
        world.reseed();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(arg) = args.first() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("koi-screensaver {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!(
                    "koi-screensaver {}\n\nThe koi pond screensaver for Omarchy. Press any key to leave.\n\n\
                     Wayland uses native ASCII tiles; --terminal selects the ANSI fallback.\n\n\
                     Environment: KOI_FPS, KOI_SIZE=COLSxROWS, KOI_FEED, KOI_FOCUS_CLASS,\n\
                     KOI_STATS=path, KOI_EXIT_AFTER=seconds, KOI_DUMP_FRAMES=path, KOI_DUMP_SECONDS",
                    env!("CARGO_PKG_VERSION")
                );
                return;
            }
            "--terminal" => {}
            other => {
                eprintln!("koi-screensaver: unknown argument {other:?} (try --help)");
                std::process::exit(2);
            }
        }
    }

    let headless_dump = std::env::var("KOI_DUMP_FRAMES").is_ok();
    let use_native = !headless_dump
        && args.first().map(String::as_str) != Some("--terminal")
        && std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !headless_dump && !use_native && !std::io::stdout().is_terminal() {
        eprintln!(
            "koi-screensaver: stdout is not a terminal (set KOI_DUMP_FRAMES=path to dump a frame)"
        );
        std::process::exit(1);
    }

    let fps = env_f64("KOI_FPS", 60.0).clamp(5.0, 240.0);
    let exit_after = env_f64("KOI_EXIT_AFTER", 0.0);
    let feed_every = env_f64("KOI_FEED", 0.0);
    let focus_class =
        std::env::var("KOI_FOCUS_CLASS").unwrap_or_else(|_| "org.omarchy.screensaver".to_string());
    let stats_path = std::env::var("KOI_STATS").unwrap_or_default();
    let seed = std::env::var("KOI_SEED")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x5EED_1234)
        });

    let mut native = if use_native {
        // Let the launcher's Foot host finish mapping before its child takes focus. Otherwise a
        // late fullscreen transition in Foot can steal focus back and immediately dismiss the pond.
        if std::io::stdout().is_terminal() {
            settled_size();
        }
        let info = window_info();
        let (w, h) = info
            .map(|i| (i.width as u32, i.height as u32))
            .unwrap_or((1280, 720));
        Some(native::Native::new(w, h).unwrap_or_else(|e| {
            eprintln!("koi-screensaver: native canvas: {e}");
            std::process::exit(1);
        }))
    } else {
        None
    };
    let creator = native.as_ref().map(native::Native::texture_creator);
    let mut atlas = creator
        .as_ref()
        .map(|c| native::GlyphAtlas::new(c).expect("glyph atlas"));
    let mut canvas_size = native.as_ref().map(native::Native::size);

    let forced_aspect = env_f64("KOI_CELL_ASPECT", 0.0);
    let mut renderer = PondRenderer::new();
    if forced_aspect > 0.0 {
        renderer.set_cell_aspect(forced_aspect);
    }
    if let Some((w, h)) = canvas_size {
        renderer.resize_canvas(w as f64, h as f64);
    } else {
        let (cols, rows) = settled_size();
        renderer.resize(cols, rows);
    }
    let (cols, rows) = (renderer.frame.cols, renderer.frame.rows);
    let mut world = World::new(seed);
    if let Some((w, h)) = canvas_size {
        world.resize(w as f64, h as f64);
    } else {
        world.resize(renderer.width(), renderer.height());
    }

    if let Ok(path) = std::env::var("KOI_DUMP_FRAMES") {
        // Debug/tests: simulate a few seconds, write a plain-text frame, exit.
        let seconds = env_f64("KOI_DUMP_SECONDS", 6.0);
        let steps = (seconds * 60.0) as usize;
        for _ in 0..steps {
            world.step(STEP);
        }
        renderer.render(&world, STEP);
        let _ = std::fs::write(path, renderer.frame.to_text());
        return;
    }

    let mut terminal = Terminal::new(cols, rows);
    let saved_stty = if native.is_none() {
        terminal.start();
        let saved = stty(&["-g"]);
        stty(&["raw", "-echo"]);
        saved
    } else {
        None
    };

    unsafe {
        signal(SIGWINCH, on_winch);
        for sig in [SIGHUP, SIGINT, SIGQUIT, SIGTERM] {
            signal(sig, on_quit);
        }
    }

    // Any byte on stdin means "wake up".
    if native.is_none() {
        std::thread::spawn(|| {
            let stdin = io::stdin();
            let mut lock = stdin.lock();
            let mut buf = [0u8; 1];
            loop {
                match lock.read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        QUIT.store(true, Ordering::SeqCst);
                        break;
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
    }

    let started = Instant::now();
    let mut last = Instant::now();
    let mut acc = 0.0f64;
    let mut frames: u64 = 0;
    let mut frames_since_stat: u64 = 0;
    let mut stat_at = Instant::now();
    let mut focus_checked = Instant::now();
    let mut focus_lost = 0;
    let mut rng: u64 = seed ^ 0x2545_F491_4F6C_DD1D;
    let mut rand = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / 9_007_199_254_740_992.0
    };
    let mut next_feed = 12.0 + rand() * 10.0;

    // The pty is usually still 80x24 for a moment while the compositor maps the window.
    let settle_until = started + Duration::from_secs(3);
    let mut size_checked = Instant::now();

    while !QUIT.load(Ordering::SeqCst) {
        if let Some(display) = &mut native {
            if display.quit_requested() {
                break;
            }
            let (w, h) = display.size();
            if canvas_size != Some((w, h)) {
                let before = renderer.width() * renderer.height();
                renderer.resize_canvas(w as f64, h as f64);
                world.resize(w as f64, h as f64);
                if !(0.8..=1.25).contains(&(renderer.width() * renderer.height() / before)) {
                    world.reseed();
                }
                canvas_size = Some((w, h));
            }
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.1);
        last = now;
        acc += dt;
        let mut steps = 0;
        while acc >= STEP && steps < MAX_STEPS {
            world.step(STEP);
            acc -= STEP;
            steps += 1;
        }
        if steps == MAX_STEPS {
            acc = 0.0;
        }

        if world.time >= next_feed {
            world.feed(
                world.width * (0.2 + rand() * 0.6),
                world.height * (0.25 + rand() * 0.5),
            );
            next_feed = world.time
                + if feed_every > 0.0 {
                    feed_every * (0.7 + rand() * 0.6)
                } else {
                    18.0 + rand() * 16.0
                };
        }

        renderer.render(&world, STEP);
        if let (Some(display), Some(atlas)) = (&mut native, &mut atlas) {
            if let Err(e) = display.paint(&renderer, atlas) {
                eprintln!("koi-screensaver: paint: {e}");
                break;
            }
        } else {
            terminal.paint(&renderer.frame);
        }
        frames += 1;
        frames_since_stat += 1;

        if native.is_none() && WINCH.swap(false, Ordering::SeqCst) {
            let (cols, rows) = size();
            apply_size(&mut renderer, &mut terminal, &mut world, cols, rows);
        }

        // The stock screensaver quits when its window stops being focused.
        if native.is_none() && focus_checked.elapsed() >= Duration::from_secs(1) {
            focus_checked = Instant::now();
            if let Some(info) = window_info() {
                // A cell is rarely 1:2: while the window and pty settle, keep measuring the real cell
                // so the pond keeps the physical aspect ratio of the screen.
                if now - started < Duration::from_secs(10)
                    && forced_aspect <= 0.0
                    && info.width > 1.0
                    && info.height > 1.0
                {
                    let aspect =
                        (info.height / terminal.rows as f64) / (info.width / terminal.cols as f64);
                    if (0.8..=4.0).contains(&aspect)
                        && (aspect - renderer.cell_aspect()).abs() > 0.03
                    {
                        renderer.set_cell_aspect(aspect);
                        renderer.resize(terminal.cols, terminal.rows);
                        world.resize(renderer.width(), renderer.height());
                    }
                }
                if !focus_class.is_empty() && now - started > Duration::from_secs(2) {
                    if info.class == focus_class {
                        focus_lost = 0;
                    } else {
                        focus_lost += 1;
                        if focus_lost >= 2 {
                            QUIT.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
        }

        if !stats_path.is_empty() && stat_at.elapsed() >= Duration::from_secs(5) {
            let seconds = stat_at.elapsed().as_secs_f64();
            let line = format!(
                "{{\"at\":\"{}\",\"uptime_s\":{:.1},\"fps\":{:.1},\"frames\":{},\"koi\":{},\"pellets\":{},\"ripples\":{},\"cells_painted\":{},\"bytes_written\":{},\"cols\":{},\"rows\":{},\"backend\":\"{}\",\"pixel_size\":{:?}}}\n",
                iso_now(),
                started.elapsed().as_secs_f64(),
                frames_since_stat as f64 / seconds,
                frames,
                world.koi.len(),
                world.pellets.len(),
                world.ripples.len(),
                terminal.cells_painted,
                terminal.bytes_written,
                renderer.frame.cols,
                renderer.frame.rows,
                if native.is_some() { "wayland" } else { "ansi" },
                native.as_ref().map(|n| { let (w, h) = n.pixel_size(); [w, h] }).unwrap_or([0, 0]),
            );
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&stats_path)
            {
                let _ = f.write_all(line.as_bytes());
            }
            frames_since_stat = 0;
            stat_at = Instant::now();
        }

        if exit_after > 0.0 && started.elapsed().as_secs_f64() >= exit_after {
            break;
        }

        // Fallback for a pty that never sends SIGWINCH while the window settles.
        if native.is_none()
            && now < settle_until
            && size_checked.elapsed() >= Duration::from_millis(250)
        {
            size_checked = Instant::now();
            let (cols, rows) = size();
            if cols != terminal.cols || rows != terminal.rows {
                apply_size(&mut renderer, &mut terminal, &mut world, cols, rows);
            }
        }

        let budget = Duration::from_secs_f64(1.0 / fps);
        let spent = now.elapsed();
        if spent < budget {
            let remaining = budget - spent;
            if remaining > Duration::from_micros(400) {
                std::thread::sleep(remaining);
            }
        }
    }

    terminal.stop();
    if let Some(saved) = saved_stty {
        stty(&[saved.as_str()]);
    }
}

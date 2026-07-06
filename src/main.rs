mod event_finder;

use std::{fs::OpenOptions, io::Write, os::fd::AsRawFd, time::{Duration, Instant}};
use evdev::{uinput::VirtualDevice, *};

#[derive(PartialEq, Clone, Copy)]
enum Brightness {
    ON = 0x01,
    LOW = 0x42,
    MID = 0x45,
    HIGH = 0x48,
    OFF = 0x00,
}

struct Touchpad {
    numpad: VirtualDevice,
    dev_id: u32,
    max_x: i32,
    max_y: i32,
    x: i32,
    y: i32,
    numlock: bool,
    brightness: Brightness,
    touch_start: Option<Instant>,
}

const ROWS: usize = 4;
const COLS: usize = 5;
const I2C_SLAVE_FORCE: libc::c_ulong = 0x0706;
const TOUCH_DURATION_S: f64 = 0.5;

const NUMPAD_LAYOUT: [[KeyCode; 5]; 4] = [
    [KeyCode::KEY_KP7, KeyCode::KEY_KP8,   KeyCode::KEY_KP9,   KeyCode::KEY_KPSLASH,    KeyCode::KEY_BACKSPACE],
    [KeyCode::KEY_KP4, KeyCode::KEY_KP5,   KeyCode::KEY_KP6,   KeyCode::KEY_KPASTERISK, KeyCode::KEY_BACKSPACE],
    [KeyCode::KEY_KP1, KeyCode::KEY_KP2,   KeyCode::KEY_KP3,   KeyCode::KEY_MINUS,      KeyCode::KEY_5], // %
    [KeyCode::KEY_KP0, KeyCode::KEY_KPDOT, KeyCode::KEY_ENTER, KeyCode::KEY_KPPLUS,     KeyCode::KEY_KPEQUAL]
];

fn main() {
    let (mouse, id) = match event_finder::find_event() {
        Some((i, x)) => (i, x),
        None => {
            eprintln!("[!] Error: finding event and touchpad id!");
            return;
        }
    };

    let mut device = match Device::open(format!("/dev/input/event{}", mouse)) {
        Ok(x) => x,
        Err(err) => {
            eprintln!("[!] Error: {err}!");
            return;
        }
    };

    let mut keys = AttributeSet::<KeyCode>::new();
    keys.insert(KeyCode::KEY_NUMLOCK);
    keys.insert(KeyCode::KEY_LEFTSHIFT);
    for i in NUMPAD_LAYOUT {
        for j in i {
            keys.insert(j);
        }
    }

    let numpad = match VirtualDevice::builder() {
        Ok(x) => match x.name("Asus Numpad") .with_keys(&keys) {
            Ok(y) => match y.build() {
                Ok(z) => z,
                Err(err) => {
                    eprintln!("[!] Error: {err}!");
                    return;
                }
            },
            Err(err) => {
                eprintln!("[!] Error: {err}!");
                return;
            },
        },
        Err(err) => {
            eprintln!("[!] Error: {err}!");
            return;
        },
    };

    let (max_x, max_y) = {
        let mut max_x = 0;
        let mut max_y = 0;
        for (axis, info) in device.get_absinfo().unwrap() {
            match axis {
                AbsoluteAxisCode::ABS_MT_POSITION_X => {
                    max_x = info.maximum();
                },
                AbsoluteAxisCode::ABS_MT_POSITION_Y => {
                    max_y = info.maximum();
                },
                _ => {}
            }
        }

        (max_x, max_y)
    };

    let mut touchpad_conf = Touchpad {
        numpad,
        dev_id: id,
        max_x,
        max_y,
        x: 0,
        y: 0,
        numlock: false,
        brightness: Brightness::OFF,
        touch_start: None
    };

    loop {
        for event in device.fetch_events().unwrap() {
            match event.destructure() {
                EventSummary::Key(_, KeyCode::BTN_TOOL_FINGER, value) => {
                    match value {
                        1 => {
                            touchpad_conf.touch_start = Some(Instant::now());
                        },
                        0 => {
                            handle_numpad(&mut touchpad_conf);
                            touchpad_conf.touch_start = None;
                        },
                        _ => {}
                    }
                },
                EventSummary::AbsoluteAxis(_, axis, value) => {
                    match axis {
                        AbsoluteAxisCode::ABS_MT_POSITION_X => {
                            touchpad_conf.x = value;
                            continue;
                        },
                        AbsoluteAxisCode::ABS_MT_POSITION_Y => {
                            touchpad_conf.y = value;
                            continue;
                        },
                        _ => {}
                    }
                },
                _ => {}
            }
        }
    }
}

fn handle_numpad(touchpad: &mut Touchpad) {
    /**** Top Left ****/
    if ((touchpad.x as f64) < 0.06 * (touchpad.max_x as f64)) && ((touchpad.y as f64) < 0.07 * (touchpad.max_y as f64)) {
        if let None = touchpad.touch_start {
            return;
        }

        let elapsed = touchpad.touch_start.unwrap().elapsed(); // Safe unwrap
        if elapsed < Duration::from_secs_f64(TOUCH_DURATION_S) {
            return;
        }

        touchpad.brightness = match touchpad.brightness {
            Brightness::LOW => Brightness::MID,
            Brightness::MID => Brightness::HIGH,
            Brightness::HIGH => Brightness::LOW,
            _ => Brightness::LOW,
        };

        change_brightness(touchpad);

        return;
    }

    /**** Top Right ****/
    if ((touchpad.x as f64) > 0.95 * (touchpad.max_x as f64)) && ((touchpad.y as f64) < 0.09 * (touchpad.max_y as f64)) {
        if let None = touchpad.touch_start {
            return;
        }

        let elapsed = touchpad.touch_start.unwrap().elapsed(); // Safe unwrap
        if elapsed < Duration::from_secs_f64(TOUCH_DURATION_S) {
            return;
        }

        match touchpad.numlock {
            true => {
                touchpad.brightness = Brightness::OFF;
                change_brightness(touchpad);
                touchpad.numpad.emit(&[
                    InputEvent::new(EventType::KEY.0, KeyCode::KEY_NUMLOCK.0, 1),
                    InputEvent::new(EventType::KEY.0, KeyCode::KEY_NUMLOCK.0, 0)
                ]).unwrap();
                touchpad.numlock = false;
            },
            false => {
                touchpad.brightness = Brightness::LOW;
                change_brightness(touchpad);
                touchpad.brightness = Brightness::ON;
                change_brightness(touchpad);
                touchpad.numpad.emit(&[
                    InputEvent::new(EventType::KEY.0, KeyCode::KEY_NUMLOCK.0, 1),
                    InputEvent::new(EventType::KEY.0, KeyCode::KEY_NUMLOCK.0, 0)
                ]).unwrap();
                touchpad.numlock = true;
            }
        }
        return;
    }

    if !touchpad.numlock {
        return;
    }

    let col = f64::floor(COLS as f64 * touchpad.x as f64 / (touchpad.max_x as f64 + 1.0)) as usize;
    let row = f64::floor((ROWS as f64 * touchpad.y as f64 / (touchpad.max_y as f64)) - 0.0) as usize;

    let key = NUMPAD_LAYOUT[row][col];

    match key {
        KeyCode::KEY_5 => touchpad.numpad.emit(&[
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTSHIFT.0, 1),
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_5.0, 1),
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_5.0, 0),
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTSHIFT.0, 0),
            ]).unwrap(),

        KeyCode::KEY_BACKSPACE => {},

        _ => touchpad.numpad.emit(&[
            InputEvent::new(EventType::KEY.0, key.0, 1),
            InputEvent::new(EventType::KEY.0, key.0, 0),
        ]).unwrap()
    }
}

fn change_brightness(touchpad: &Touchpad) {
    let path = format!("/dev/i2c-{}", touchpad.dev_id);
    dbg!(&path);
    let dev = OpenOptions::new()
        .write(true)
        .read(true)
        .open(&path);

    let mut dev = match dev {
        Ok(x) => x,
        Err(err) => {
            eprintln!("[!] Error: {}", err);
            return;
        }
    };

    let fd = dev.as_raw_fd();

    let ret = unsafe {
        libc::ioctl(fd, I2C_SLAVE_FORCE, 0x15)
    };

    if ret < 0 {
        eprintln!("[!] Error: failed writting to {}", path);
        return;
    }

    let brightness = touchpad.brightness as u8;

    let data = [ 0x05, 0x00, 0x3d, 0x03, 0x06, 0x00, 0x07, 0x00, 0x0d, 0x14, 0x03, brightness, 0xad ]; // 12 byte

    if let Err(_) = dev.write_all(&data) {
        eprintln!("[!] Error: writting to {}", path);
    }
}

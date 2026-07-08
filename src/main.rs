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

const ROWS: usize = 5;
const COLS: usize = 4;
const I2C_SLAVE_FORCE: libc::c_ulong = 0x0706;
const TOUCH_DURATION_S: f64 = 0.5;

struct Touchpad {
    numpad: VirtualDevice,
    dev_id: u32,
    max_x: i32,
    max_y: i32,
    x: i32,
    y: i32,
    row: usize,
    col: usize,
    numlock: bool,
    brightness: Brightness,
    touch_start: Option<Instant>,
    backspace: bool,
}

impl Touchpad {
    fn new(numpad: VirtualDevice, id: u32, x: i32, y: i32) -> Self {
        Touchpad {
            numpad,
            dev_id: id,
            max_x: x,
            max_y: y,
            x: 0,
            y: 0,
            row: 0,
            col: 0,
            numlock: false,
            brightness: Brightness::OFF,
            touch_start: None,
            backspace: false,
        }
    }

    fn key(&mut self) {
        let col = f64::floor(COLS as f64 * self.x as f64 / (self.max_x as f64 + 1.0)) as usize;
        let row = f64::floor((ROWS as f64 * self.y as f64 / (self.max_y as f64)) - 0.0) as usize;

        self.col = match col {
            i if i >= COLS => self.col,
            _ => col
        };

        self.row = match row {
            i if i >= ROWS => self.row,
            _ => row
        };
    }
}

const NUMPAD_LAYOUT: [[KeyCode; ROWS]; COLS] = [
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

    let mut touchpad = Touchpad::new(numpad, id, max_x, max_y);

    loop {
        for event in device.fetch_events().unwrap() {
            match event.destructure() {
                EventSummary::Key(_, KeyCode::BTN_TOOL_FINGER, value) => {
                    match value {
                        1 => {
                            if let true = touchpad.numlock {
                                touchpad.key();
                                if let KeyCode::KEY_BACKSPACE = NUMPAD_LAYOUT[touchpad.row][touchpad.col] {
                                    touchpad.numpad.emit(&[
                                        InputEvent::new(EventType::KEY.0, KeyCode::KEY_BACKSPACE.0, 1),
                                    ]).unwrap();

                                    touchpad.backspace = true;
                                }
                            }
                            touchpad.touch_start = Some(Instant::now());
                        },

                        0 => {
                            if let true = touchpad.backspace {
                                touchpad.numpad.emit(&[
                                    InputEvent::new(EventType::KEY.0, KeyCode::KEY_BACKSPACE.0, 0),
                                ]).unwrap();
                                touchpad.backspace = false;
                            }

                            handle_numpad(&mut touchpad);
                            touchpad.touch_start = None;
                        },

                        _ => {}
                    }
                },
                EventSummary::AbsoluteAxis(_, axis, value) => {
                    match axis {
                        AbsoluteAxisCode::ABS_MT_POSITION_X => {
                            touchpad.x = value;
                            continue;
                        },
                        AbsoluteAxisCode::ABS_MT_POSITION_Y => {
                            touchpad.y = value;
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
        dbg!("Top left");
        if let None = touchpad.touch_start {
            return;
        }

        let elapsed = touchpad.touch_start.unwrap().elapsed(); // Safe unwrap
        if elapsed < Duration::from_secs_f64(TOUCH_DURATION_S) {
            return;
        }

        touchpad.brightness = match touchpad.brightness {
            Brightness::OFF => Brightness::LOW,
            Brightness::ON => Brightness::LOW,
            Brightness::LOW => Brightness::MID,
            Brightness::MID => Brightness::HIGH,
            Brightness::HIGH => Brightness::OFF, // Switch it off
        };

        change_brightness(touchpad);
        if Brightness::OFF == touchpad.brightness && touchpad.numlock == true {
            touchpad.numpad.emit(&[
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_NUMLOCK.0, 1),
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_NUMLOCK.0, 0)
            ]).unwrap();
            touchpad.numlock = false;
        }

        return;
    }

    /**** Top Right ****/
    if ((touchpad.x as f64) > 0.95 * (touchpad.max_x as f64)) && ((touchpad.y as f64) < 0.09 * (touchpad.max_y as f64)) {
        dbg!("Top right");
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
                // Set the light mode
                touchpad.brightness = Brightness::LOW;
                change_brightness(touchpad);
                // Set it to On
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

    let key = NUMPAD_LAYOUT[touchpad.row][touchpad.col];

    match key {
        KeyCode::KEY_5 => touchpad.numpad.emit(&[
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTSHIFT.0, 1),
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_5.0, 1),
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_5.0, 0),
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTSHIFT.0, 0),
        ]).unwrap(),

        KeyCode::KEY_BACKSPACE => { /* Handled above */ },

        _ => touchpad.numpad.emit(&[
            InputEvent::new(EventType::KEY.0, key.0, 1),
            InputEvent::new(EventType::KEY.0, key.0, 0),
        ]).unwrap()
    }
}

fn change_brightness(touchpad: &Touchpad) {
    let path = format!("/dev/i2c-{}", touchpad.dev_id);
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

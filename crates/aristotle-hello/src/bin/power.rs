//! Prints battery status and ramps the frontlight up and back down, restoring
//! the original brightness. Checks the power module against the real device.

use std::thread;
use std::time::Duration;

use aristotle_hal::power::{Battery, Frontlight};

fn main() -> std::io::Result<()> {
    let battery = Battery::open()?;
    println!("battery: {:?}", battery.status()?);

    let light = Frontlight::open()?;
    let original = light.brightness()?;
    println!("frontlight: {original} of {}", light.max());
    for level in [0, 100, 400, 1000, 400, 100, 0] {
        light.set_brightness(level)?;
        println!("  set {level:>4}, reads {}", light.brightness()?);
        thread::sleep(Duration::from_millis(600));
    }
    light.set_brightness(original)?;
    println!("restored {}", light.brightness()?);
    Ok(())
}

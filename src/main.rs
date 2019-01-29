extern crate alsa;
#[macro_use]
extern crate nom;

use alsa::{mixer, PollDescriptors};
use std::io::prelude::*;
use std::net::TcpStream;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::{env, error, panic, thread, time};

pub mod nad_protocol;
use nad_protocol::{parse_frames, OpCode, ReceiverFrame};

fn open_audio_ctl(name: &str) -> Result<alsa::Ctl, Box<error::Error>> {
    let ctl = alsa::Ctl::new(&name, false)?;
    ctl.subscribe_events(true)?;
    Ok(ctl)
}

fn open_mixer(name: &str) -> Result<alsa::Mixer, Box<error::Error>> {
    Ok(alsa::Mixer::new(&name, false)?)
}

fn listen(name: String, f: Box<Fn(f64) + Send>) -> Result<(), Box<error::Error>> {
    let mixer = open_audio_ctl(&name)?;
    let mut fds = mixer.get()?;
    mixer.fill(&mut fds).unwrap();
    loop {
        alsa::poll::poll(&mut fds, 10000)?;
        mixer.revents(&fds)?;
        if let Ok(Some(event)) = mixer.read() {
            let mask = event.get_mask();
            if mask.0 & 1 == 1 {
                f(alsa_volume(&name)?);
            }
        }
    }
}

fn alsa_volume(name: &str) -> Result<f64, Box<error::Error>> {
    let mixer = open_mixer(name)?;
    let selem = mixer.find_selem(&mixer::SelemId::new("Master", 0)).unwrap();
    let channel = mixer::SelemChannelId::mono();

    let (min, max) = selem.get_playback_volume_range();
    match selem.get_playback_volume(channel) {
        Ok(volume) => Ok((volume - min) as f64 / (max - min) as f64),
        Err(error) => Err(error.into()),
    }
}

fn percent_to_range(volume: f64, min: i64, max: i64) -> i64 {
    let range = (max - min) as f64;
    (min as f64 + volume * range).round() as i64
}

fn percent_to_receiver_volume(volume: f64) -> u8 {
    (volume * 180.0).round() as u8
}

fn receiver_volume_to_percent(volume: u8) -> f64 {
    f64::from(volume) / 180.0
}

fn set_alsa_volume(name: String, volume: f64) -> Result<(), Box<error::Error>> {
    let mixer = open_mixer(&name)?;
    let selem = mixer.find_selem(&mixer::SelemId::new("Master", 0)).unwrap();
    let (min, max) = selem.get_playback_volume_range();
    let alsa_volume = percent_to_range(volume, min, max);
    println!(
        "update alsa volume {} {} {:?}",
        volume,
        percent_to_receiver_volume(volume),
        alsa_volume
    );
    match selem.set_playback_volume_all(alsa_volume) {
        Ok(_) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn set_receiver_volume(stream: &mut TcpStream, volume: f64) -> Result<(), Box<error::Error>> {
    let receiver_volume = percent_to_receiver_volume(volume);
    stream.write_all(&[0, 1, 2, 4, receiver_volume])?;
    Ok(())
}

fn last_volume(frames: &[ReceiverFrame]) -> Option<&ReceiverFrame> {
    frames.iter().fold(None, |volume, frame| {
        if frame.command == OpCode::Volume {
            Some(frame)
        } else {
            volume
        }
    })
}

fn read_stream(stream: &mut TcpStream) -> Result<f64, Box<error::Error>> {
    let mut buf = [0; 20];
    let bytes = stream.read(&mut buf)?;
    if bytes > 4 {
        let result = parse_frames(&buf);
        match result {
            Ok((_, frames)) => {
                let volume =
                    last_volume(&frames).map(|frame| receiver_volume_to_percent(frame.payload));
                match volume {
                    Some(volume) => Ok(volume),
                    None => Err("no volume found".into()),
                }
            }
            Err(_) => Err("error parsing stream".into()),
        }
    } else {
        Err("failed reading stream".into())
    }
}

fn poll_receiver_volume(stream: &mut TcpStream) -> Result<(), Box<error::Error>> {
    stream.write_all(&[0, 1, 2, 2, 4])?;
    Ok(())
}

enum Command {
    PollVolume,
    ReceiverVolumeChange { volume: f64 },
    AlsaVolumeChange { volume: f64 },
}

fn can_update(last_update: time::SystemTime, last_volume: f64, volume: f64) -> bool {
    let duration = last_update
        .elapsed()
        .expect("Could not compute duration since last update");
    let diff = (volume - last_volume).abs();
    duration.as_secs() > 2 && diff > 0.005
}

fn sync_volumes(
    name_clone: String,
    mut stream: TcpStream,
    rx: Receiver<Command>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_volume = 0.0;
        let mut last_alsa_update = time::SystemTime::UNIX_EPOCH;
        let mut last_receiver_update = time::SystemTime::UNIX_EPOCH;

        loop {
            match rx.recv() {
                Ok(Command::ReceiverVolumeChange { volume }) => {
                    if can_update(last_alsa_update, last_volume, volume) {
                        last_receiver_update = time::SystemTime::now();
                        last_volume = volume;
                        set_alsa_volume(name_clone.clone(), volume)
                            .expect("Could not change ALSA volume");
                    }
                }
                Ok(Command::PollVolume) => {
                    poll_receiver_volume(&mut stream).expect("Polling volume failed");
                }
                Ok(Command::AlsaVolumeChange { volume }) => {
                    if can_update(last_receiver_update, last_volume, volume) {
                        last_alsa_update = time::SystemTime::now();
                        last_volume = volume;
                        set_receiver_volume(&mut stream, volume)
                            .expect("Could not change receiver volume");
                    }
                }
                Err(message) => eprintln!("Error: {}", message),
            }
        }
    })
}

fn poll_volumes(tx: Sender<Command>) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        tx.send(Command::PollVolume)
            .expect("Could not send volume poll");
        thread::sleep(time::Duration::from_millis(10000));
    })
}

fn listen_receiver(mut stream_reader: TcpStream, tx: Sender<Command>) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        let result = read_stream(&mut stream_reader);
        match result {
            Ok(volume) => {
                tx.send(Command::ReceiverVolumeChange { volume })
                    .expect("Could not send receiver volume change");;
            }
            Err(_) => {
                eprintln!("Error");
            }
        }
    })
}

fn receiver_connect(address: String) -> Result<TcpStream, Box<error::Error>> {
    let stream = TcpStream::connect(address.to_string() + ":50001")?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

fn main() -> Result<(), Box<error::Error>> {
    panic::set_hook(Box::new(|error| {
        eprintln!("Error: {:?}", error);
        std::process::exit(1);
    }));

    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        println!("Usage: 'cargo run --release CARD_NAME RECEIVER_ADDRESS'");
        Err("No card name specified")?
    }

    let args: Vec<_> = std::env::args().collect();
    let name = (&args[1]).to_string();
    let address = (&args[2]).to_string();
    let stream = receiver_connect(address)?;
    let (tx, rx): (Sender<Command>, Receiver<Command>) = channel();

    poll_volumes(tx.clone());
    listen_receiver(stream.try_clone().expect("clone failed..."), tx.clone());
    sync_volumes(name.clone(), stream, rx);

    listen(
        name.clone(),
        Box::new(move |volume| {
            tx.send(Command::AlsaVolumeChange { volume })
                .expect("Could not send volume change command");;
        }),
    )?;

    Ok(())
}

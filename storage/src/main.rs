extern crate failure;
//extern crate gio;
extern crate glib;

#[macro_use]
extern crate failure_derive;
extern crate gstreamer as gst;
extern crate gstreamer_rtsp as gst_rtsp;
extern crate gstreamer_rtsp_server as gst_rtsp_server;
extern crate gstreamer_rtsp_server_sys as gst_rtsp_server_sys;

use serde_derive::Deserialize;

use failure::Error;
use std::env;
use std::path::Path;

use gst_rtsp::*;
use gst_rtsp_server::prelude::*;
use gst_rtsp_server::*;

// ---- Custom Exceptions ----

#[derive(Debug, Fail)]
#[fail(display = "Could not get mount points")]
struct NoMountPoints;

#[derive(Debug, Fail)]
#[fail(display = "Usage: {} CONFIG-FILE", _0)]
struct UsageError(String);

// ---- Configuration Structure ----

#[derive(Clone, Debug, Deserialize)]
struct ConfigHTTP {
    host: String,
    port: u16,
}

#[derive(Clone, Debug, Deserialize)]
struct Config {
    http: ConfigHTTP,
}

fn run(config: Config) -> Result<(), Error> {
    let main_loop = glib::MainLoop::new(None, false);
    let server = RTSPServer::new();
    let mounts = server.get_mount_points().ok_or(NoMountPoints)?;
    let factory = RTSPMediaFactory::new();

    // Port numbers are passed as strings to the gst-rtsp server
    let port = config.http.port.to_string();
    server.set_property("service", &port).unwrap();


    // Finish configuring media factory
    let line = "rtph264depay name=depay0 ! h264parse ! splitmuxsink location=video%02d.mp4 max-size-time=10000000000";
    factory.set_transport_mode(RTSPTransportMode::RECORD);
    factory.set_profiles(RTSPProfile::AVP | RTSPProfile::AVPF);
    factory.set_launch(line);

    // Mounting point for the stream
    mounts.add_factory("/test", &factory);

    let id = server.attach(None);

    println!(
        "Stream ready at rtsps://127.0.0.1:{}/test",
        server.get_bound_port()
    );

    main_loop.run();

    glib::source_remove(id);

    Ok(())
}

fn load_config(args: &Vec<String>) -> Result<Config, Error> {
    if args.len() != 2 {
        // Can't move on without the configuration file
        return Err(Error::from(UsageError(args[0].clone())));
    } else {
        // Let's try to read the file contents
        let path = Path::new(&args[1]);
        let contents = std::fs::read_to_string(path)?;
        // If file contents are readable, we then try to parse the
        // TOML string that was read from it.
        let strcontent = contents.as_str();
        let config: Config = toml::from_str(strcontent)?;
        Ok(config)
    }
}

fn main() -> Result<(), Error> {
    gst::init()?;

    let args: Vec<String> = env::args().collect();
    let config: Config = load_config(&args)?;
    run(config)
}

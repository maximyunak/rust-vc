use common::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use opus::{Application, Bitrate, Channels, Decoder, Encoder};
use std::collections::{HashMap, VecDeque};
use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PREBUF: usize = FRAME * 3; // 60 мс перед началом воспроизведения
const MAXBUF: usize = FRAME * 12; // не копим больше 240 мс

struct Peer {
    name: String,
    dec: Decoder,
    buf: VecDeque<f32>,
    playing: bool,
    last_seq: Option<u16>,
    last_seen: Instant,
    last_voice: Option<Instant>,
}

#[derive(Default)]
struct Mixer {
    peers: HashMap<u32, Peer>,
}

impl Mixer {
    /// Один сэмпл 48 кГц, смешанный от всех участников.
    fn pull(&mut self) -> f32 {
        let mut s = 0.0;
        for p in self.peers.values_mut() {
            if !p.playing {
                continue;
            }
            match p.buf.pop_front() {
                Some(v) => s += v,
                None => p.playing = false,
            }
        }
        s.clamp(-1.0, 1.0)
    }
}

pub struct Call {
    mixer: Arc<Mutex<Mixer>>,
    pub muted: Arc<AtomicBool>,
    pub deaf: Arc<AtomicBool>,
    pub denoise: Arc<AtomicBool>,
    pub echo: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    sock: Arc<UdpSocket>,
    room: u64,
    user: u32,
    _in: Stream,
    _out: Stream,
}

pub struct PeerView {
    pub name: String,
    pub speaking: bool,
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

pub fn list_devices(input: bool) -> Vec<String> {
    let host = cpal::default_host();
    let devs = if input {
        host.input_devices()
    } else {
        host.output_devices()
    };
    devs.map(|d| d.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

fn pick(host: &cpal::Host, name: Option<&str>, input: bool) -> Result<cpal::Device, String> {
    if let Some(n) = name {
        let mut devs = if input {
            host.input_devices()
        } else {
            host.output_devices()
        }
        .map_err(|e| e.to_string())?;
        if let Some(d) = devs.find(|d| d.name().ok().as_deref() == Some(n)) {
            return Ok(d);
        }
    }
    let d = if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    };
    d.ok_or_else(|| {
        if input {
            "Микрофон не найден".into()
        } else {
            "Устройство вывода не найдено".into()
        }
    })
}

fn input_stream<T>(
    dev: &cpal::Device,
    cfg: &StreamConfig,
    cap: Arc<Mutex<VecDeque<f32>>>,
) -> Result<Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let ch = cfg.channels as usize;
    let step = cfg.sample_rate.0 as f64 / RATE as f64;
    let (mut pos, mut prev) = (0.0f64, 0.0f32);
    let stream = dev
        .build_input_stream(
            cfg,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut q = cap.lock().unwrap();
                for frame in data.chunks(ch) {
                    let s = frame.iter().map(|&x| f32::from_sample(x)).sum::<f32>() / ch as f32;
                    while pos < 1.0 {
                        q.push_back(prev + (s - prev) * pos as f32);
                        pos += step;
                    }
                    pos -= 1.0;
                    prev = s;
                }
                if q.len() > RATE as usize {
                    let extra = q.len() - RATE as usize;
                    q.drain(..extra);
                }
            },
            |e| eprintln!("input: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn output_stream<T>(
    dev: &cpal::Device,
    cfg: &StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
    deaf: Arc<AtomicBool>,
    refq: Arc<Mutex<VecDeque<f32>>>,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let ch = cfg.channels as usize;
    let step = RATE as f64 / cfg.sample_rate.0 as f64;
    let (mut pos, mut prev, mut cur) = (1.0f64, 0.0f32, 0.0f32);
    let stream = dev
        .build_output_stream(
            cfg,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let mut m = mixer.lock().unwrap();
                let mut rq = refq.lock().unwrap();
                let deaf = deaf.load(Relaxed);
                for frame in data.chunks_mut(ch) {
                    while pos >= 1.0 {
                        pos -= 1.0;
                        prev = cur;
                        let v = m.pull();
                        cur = if deaf { 0.0 } else { v };
                        rq.push_back(cur);
                    }
                    let v = prev + (cur - prev) * pos as f32;
                    pos += step;
                    let s = T::from_sample(v);
                    for o in frame.iter_mut() {
                        *o = s;
                    }
                }
            },
            |e| eprintln!("output: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn handle(mixer: &Mutex<Mixer>, kind: u8, user: u32, payload: &[u8], pcm: &mut [f32]) {
    let mut m = mixer.lock().unwrap();
    if kind == LEAVE {
        m.peers.remove(&user);
        return;
    }
    if !m.peers.contains_key(&user) {
        let Ok(dec) = Decoder::new(RATE, Channels::Mono) else {
            return;
        };
        m.peers.insert(
            user,
            Peer {
                name: "...".into(),
                dec,
                buf: VecDeque::new(),
                playing: false,
                last_seq: None,
                last_seen: Instant::now(),
                last_voice: None,
            },
        );
    }
    let p = m.peers.get_mut(&user).unwrap();
    p.last_seen = Instant::now();
    match kind {
        JOIN | PING => p.name = String::from_utf8_lossy(payload).into_owned(),
        AUDIO => {
            if payload.len() < 3 {
                return;
            }
            let seq = u16::from_be_bytes([payload[0], payload[1]]);
            if let Some(last) = p.last_seq {
                let d = seq.wrapping_sub(last);
                if d == 0 || d > 32768 {
                    return; // дубликат или опоздавший пакет
                }
            }
            p.last_seq = Some(seq);
            let Ok(n) = p.dec.decode_float(&payload[2..], pcm, false) else {
                return;
            };
            let pcm = &pcm[..n];
            if rms(pcm) > 0.01 {
                p.last_voice = Some(Instant::now());
            }
            p.buf.extend(pcm.iter().copied());
            if p.buf.len() > MAXBUF {
                let extra = p.buf.len() - MAXBUF;
                p.buf.drain(..extra);
            }
            if !p.playing && p.buf.len() >= PREBUF {
                p.playing = true;
            }
        }
        _ => {}
    }
}

impl Call {
    pub fn start(
        server: &str,
        room: u64,
        name: &str,
        input: Option<&str>,
        output: Option<&str>,
    ) -> Result<Call, String> {
        let addr = server
            .to_socket_addrs()
            .map_err(|e| format!("Адрес сервера: {e}"))?
            .find(|a| a.is_ipv4())
            .ok_or("Адрес сервера не найден")?;
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        sock.connect(addr).map_err(|e| e.to_string())?;
        sock.set_read_timeout(Some(Duration::from_millis(200))).ok();
        let sock = Arc::new(sock);
        let user = rand_u64() as u32;

        let host = cpal::default_host();
        let in_dev = pick(&host, input, true)?;
        let out_dev = pick(&host, output, false)?;

        let cap: Arc<Mutex<VecDeque<f32>>> = Arc::default();
        let mixer: Arc<Mutex<Mixer>> = Arc::default();
        let muted = Arc::new(AtomicBool::new(false));
        let deaf = Arc::new(AtomicBool::new(false));
        let denoise = Arc::new(AtomicBool::new(true));
        let level = Arc::new(AtomicU32::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        let refq: Arc<Mutex<VecDeque<f32>>> = Arc::default();
        let echo = Arc::new(AtomicBool::new(true));


        let sup = in_dev.default_input_config().map_err(|e| e.to_string())?;
        let cfg = sup.config();
        let in_stream = match sup.sample_format() {
            SampleFormat::F32 => input_stream::<f32>(&in_dev, &cfg, cap.clone()),
            SampleFormat::I16 => input_stream::<i16>(&in_dev, &cfg, cap.clone()),
            SampleFormat::U16 => input_stream::<u16>(&in_dev, &cfg, cap.clone()),
            f => Err(format!("Формат микрофона не поддерживается: {f:?}")),
        }?;

        let sup = out_dev.default_output_config().map_err(|e| e.to_string())?;
        let cfg = sup.config();
        let out_stream = match sup.sample_format() {
            SampleFormat::F32 => output_stream::<f32>(&out_dev, &cfg, mixer.clone(), deaf.clone(), refq.clone()),
            SampleFormat::I16 => output_stream::<i16>(&out_dev, &cfg, mixer.clone(), deaf.clone(), refq.clone()),
            SampleFormat::U16 => output_stream::<u16>(&out_dev, &cfg, mixer.clone(), deaf.clone(), refq.clone()),
            f => Err(format!("Формат вывода не поддерживается: {f:?}")),
        }?;

        let mut enc =
            Encoder::new(RATE, Channels::Mono, Application::Voip).map_err(|e| e.to_string())?;
        enc.set_bitrate(Bitrate::Bits(32_000)).ok();

        // Поток отправки: кадры 20 мс -> Opus -> UDP
        {
            let denoise = denoise.clone();
            let echo = echo.clone();
            let refq = refq.clone();
            let (sock, muted, level, stop) =
                (sock.clone(), muted.clone(), level.clone(), stop.clone());
            let name = name.trim().to_string();
            std::thread::spawn(move || {
                let hello = |kind: u8| {
                    let mut p = header(kind, room, user);
                    p.extend_from_slice(name.as_bytes());
                    let _ = sock.send(&p);
                };
                let config = aec_rs::AecConfig {
                    frame_size: FRAME,
                    filter_length: 9600,      // Corresponds to your previous 2nd argument
                    sample_rate: RATE as u32,
                    enable_preprocess: false,  // Corresponds to your previous 4th argument
                };
                let aec = aec_rs::Aec::new(&config);
                let to_i16 = |x: &[f32]| -> Vec<i16> {
                    x.iter().map(|v| (v * 32767.0).clamp(-32768.0, 32767.0) as i16).collect()
                };
                let mut dn = nnnoiseless::DenoiseState::new();
                hello(JOIN);
                refq.lock().unwrap().clear();
                let (mut seq, mut hang) = (0u16, 0u32);
                let mut last_ping = Instant::now();
                let mut out = [0u8; 1400];
                while !stop.load(Relaxed) {
                    let frame: Option<Vec<f32>> = {
                        let mut q = cap.lock().unwrap();
                        if q.len() >= FRAME {
                            Some(q.drain(..FRAME).collect())
                        } else {
                            None
                        }
                    };
                    let Some(mut f) = frame else {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    };
                    let far: Vec<f32> = {
                        let mut r = refq.lock().unwrap();
                        if r.len() > FRAME * 4 {
                            let extra = r.len() - FRAME * 2;
                            r.drain(..extra);
                        }
                        let n = r.len().min(FRAME);
                        let mut v: Vec<f32> = r.drain(..n).collect();
                        v.resize(FRAME, 0.0);
                        v
                    };
                    if echo.load(Relaxed) {
                        let rec = to_i16(&f);
                        let farx = to_i16(&far);
                        let mut clean = vec![0i16; FRAME];
                        aec.cancel_echo(&rec, &farx, &mut clean);
                        for (d, s) in f.iter_mut().zip(&clean) {
                            *d = *s as f32 / 32768.0;
                        }
                    }
                    if denoise.load(Relaxed) {
                        let mut out_buf = [0f32; 480];
                        for part in f.chunks_mut(480) {
                            // RNNoise ждёт сэмплы в масштабе i16, а не от -1 до 1
                            let inp: Vec<f32> = part.iter().map(|v| v * 32768.0).collect();
                            dn.process_frame(&mut out_buf, &inp);
                            for (d, s) in part.iter_mut().zip(out_buf.iter()) {
                                *d = s / 32768.0;
                            }
                        }
                    }
                    let muted = muted.load(Relaxed);
                    let r = if muted { 0.0 } else { rms(&f) };
                    level.store(r.to_bits(), Relaxed);
                    let active = r > 0.0015 || hang > 0;
                    if r > 0.0015 {
                        hang = 15
                    } else if hang > 0 {
                        hang -= 1
                    }
                    if active && !muted {
                        if let Ok(n) = enc.encode_float(&f, &mut out) {
                            let mut p = header(AUDIO, room, user);
                            p.extend_from_slice(&seq.to_be_bytes());
                            p.extend_from_slice(&out[..n]);
                            seq = seq.wrapping_add(1);
                            let _ = sock.send(&p);
                        }
                    }
                    if last_ping.elapsed() > Duration::from_secs(1) {
                        last_ping = Instant::now();
                        hello(PING);
                    }
                }
            });
        }

        // Поток приёма: UDP -> Opus -> буферы участников
        {
            let (sock, mixer, stop) = (sock.clone(), mixer.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                let mut pcm = vec![0f32; 5760];
                let mut last_prune = Instant::now();
                while !stop.load(Relaxed) {
                    match sock.recv(&mut buf) {
                        Ok(n) => {
                            if let Some((kind, r, u, payload)) = parse(&buf[..n]) {
                                if r == room && u != user {
                                    handle(&mixer, kind, u, payload, &mut pcm);
                                }
                            }
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(_) => std::thread::sleep(Duration::from_millis(50)),
                    }
                    if last_prune.elapsed() > Duration::from_secs(1) {
                        last_prune = Instant::now();
                        mixer
                            .lock()
                            .unwrap()
                            .peers
                            .retain(|_, p| p.last_seen.elapsed() < Duration::from_secs(5));
                    }
                }
            });
        }

        Ok(Call {
            mixer, muted, deaf, denoise, echo, level, stop, sock, room, user,
            _in: in_stream,
            _out: out_stream,
        })
    }

    pub fn peers(&self) -> Vec<PeerView> {
        let m = self.mixer.lock().unwrap();
        let mut v: Vec<PeerView> = m
            .peers
            .values()
            .map(|p| PeerView {
                name: p.name.clone(),
                speaking: p
                    .last_voice
                    .map_or(false, |t| t.elapsed() < Duration::from_millis(300)),
            })
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn me_speaking(&self) -> bool {
        f32::from_bits(self.level.load(Relaxed)) > 0.01
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        let _ = self.sock.send(&header(LEAVE, self.room, self.user));
    }
}

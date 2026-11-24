//! M6 · el MP4 del video, nativo de macOS: `AVAssetWriter` + `AVAssetWriterInputPixelBufferAdaptor`
//! con H.264 (el códec por hardware de Apple). Sin ffmpeg ni bibliotecas de terceros: lo que la Mac ya
//! trae. Recibe cuadros RGB de 8 bits, uno a uno.

use anyhow::{anyhow, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AllocAnyThread;
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor, AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaTypeVideo, AVVideoAverageBitRateKey, AVVideoCodecKey, AVVideoCodecTypeH264,
    AVVideoCompressionPropertiesKey, AVVideoHeightKey, AVVideoWidthKey,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::CMTime;
use objc2_core_video::{kCVPixelFormatType_32BGRA, CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use std::path::Path;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

/// Un MP4 abierto: se le dan cuadros RGB y se cierra.
pub struct Mp4 {
    escritor: Retained<AVAssetWriter>,
    entrada: Retained<AVAssetWriterInput>,
    adaptador: Retained<AVAssetWriterInputPixelBufferAdaptor>,
    ancho: usize,
    alto: usize,
    cps: i32,
    n: i64,
}

fn clave(k: Option<&'static NSString>) -> Result<&'static NSString> {
    k.ok_or_else(|| anyhow_tr!("AVFoundation sin la clave de video", "AVFoundation without the video key"))
}

impl Mp4 {
    /// `ruta` (se reemplaza si existe), tamaño par y cuadros por segundo. Tasa de bits: ~0,15 bit por
    /// píxel y cuadro (un 720p a 24 cps ≈ 8 Mb/s), la calidad de una cámara.
    pub fn nuevo(ruta: &Path, ancho: usize, alto: usize, cps: i32) -> Result<Mp4> {
        if ancho % 2 == 1 || alto % 2 == 1 || ancho == 0 || alto == 0 {
            bail_tr!("el video H.264 necesita ancho y alto pares ({ancho}×{alto})", "H.264 video needs even width and height ({ancho}×{alto})");
        }
        let _ = std::fs::remove_file(ruta);
        let url = NSURL::fileURLWithPath(&NSString::from_str(&ruta.to_string_lossy()));
        unsafe {
            let tipo = AVFileTypeMPEG4.ok_or_else(|| anyhow_tr!("AVFoundation sin MPEG-4", "AVFoundation without MPEG-4"))?;
            let escritor = AVAssetWriter::initWithURL_fileType_error(AVAssetWriter::alloc(), &url, tipo).map_err(|e| anyhow!("AVAssetWriter: {}", e.localizedDescription()))?;
            let bits = ((ancho * alto) as f64 * cps as f64 * 0.15) as i64;
            let compresion = NSDictionary::<NSString, AnyObject>::from_slices(&[clave(AVVideoAverageBitRateKey)?], &[&*NSNumber::new_i64(bits) as &AnyObject]);
            let ajustes = NSDictionary::<NSString, AnyObject>::from_slices(
                &[clave(AVVideoCodecKey)?, clave(AVVideoWidthKey)?, clave(AVVideoHeightKey)?, clave(AVVideoCompressionPropertiesKey)?],
                &[
                    AVVideoCodecTypeH264.ok_or_else(|| anyhow_tr!("AVFoundation sin H.264", "AVFoundation without H.264"))? as &AnyObject,
                    &*NSNumber::new_usize(ancho) as &AnyObject,
                    &*NSNumber::new_usize(alto) as &AnyObject,
                    &*compresion as &AnyObject,
                ],
            );
            let medio = AVMediaTypeVideo.ok_or_else(|| anyhow_tr!("AVFoundation sin video", "AVFoundation without video"))?;
            let entrada = AVAssetWriterInput::initWithMediaType_outputSettings(AVAssetWriterInput::alloc(), medio, Some(&ajustes));
            entrada.setExpectsMediaDataInRealTime(false);
            if !escritor.canAddInput(&entrada) {
                bail_tr!("AVAssetWriter no acepta la pista de video", "AVAssetWriter does not accept the video track");
            }
            escritor.addInput(&entrada);
            let adaptador = AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(&entrada, None);
            if !escritor.startWriting() {
                let e = escritor.error().map(|e| e.localizedDescription().to_string()).unwrap_or_default(); bail_tr!("AVAssetWriter no arrancó: {e}", "AVAssetWriter did not start: {e}");
            }
            escritor.startSessionAtSourceTime(CMTime::new(0, cps));
            Ok(Mp4 { escritor, entrada, adaptador, ancho, alto, cps, n: 0 })
        }
    }

    /// Un cuadro RGB (`ancho·alto·3` bytes, filas de arriba abajo).
    pub fn cuadro(&mut self, rgb: &[u8]) -> Result<()> {
        if rgb.len() != self.ancho * self.alto * 3 {
            let (n, e) = (rgb.len(), self.ancho * self.alto * 3); bail_tr!("cuadro de {n} bytes; se esperaban {e}", "frame of {n} bytes; {e} expected");
        }
        unsafe {
            let mut crudo: *mut CVPixelBuffer = std::ptr::null_mut();
            let r = CVPixelBufferCreate(None, self.ancho, self.alto, kCVPixelFormatType_32BGRA, None, NonNull::from(&mut crudo));
            let pb: CFRetained<CVPixelBuffer> = match NonNull::new(crudo) {
                Some(p) if r == 0 => CFRetained::from_raw(p),
                _ => bail_tr!("CVPixelBufferCreate falló ({r})", "CVPixelBufferCreate failed ({r})"),
            };
            CVPixelBufferLockBaseAddress(&pb, CVPixelBufferLockFlags(0));
            let base = CVPixelBufferGetBaseAddress(&pb) as *mut u8;
            let fila = CVPixelBufferGetBytesPerRow(&pb);
            for y in 0..self.alto {
                let d = std::slice::from_raw_parts_mut(base.add(y * fila), self.ancho * 4);
                let s = &rgb[y * self.ancho * 3..(y + 1) * self.ancho * 3];
                for x in 0..self.ancho {
                    d[x * 4] = s[x * 3 + 2];
                    d[x * 4 + 1] = s[x * 3 + 1];
                    d[x * 4 + 2] = s[x * 3];
                    d[x * 4 + 3] = 255;
                }
            }
            CVPixelBufferUnlockBaseAddress(&pb, CVPixelBufferLockFlags(0));
            let limite = Instant::now() + Duration::from_secs(30);
            while !self.entrada.isReadyForMoreMediaData() {
                if Instant::now() > limite || self.escritor.status() == AVAssetWriterStatus::Failed {
                    let e = self.escritor.error().map(|e| e.localizedDescription().to_string()).unwrap_or_default(); bail_tr!("el codificador de video no avanza: {e}", "the video encoder is stuck: {e}");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            if !self.adaptador.appendPixelBuffer_withPresentationTime(&pb, CMTime::new(self.n, self.cps)) {
                let (n, e) = (self.n, self.escritor.error().map(|e| e.localizedDescription().to_string()).unwrap_or_default()); bail_tr!("no pude añadir el cuadro {n}: {e}", "could not append frame {n}: {e}");
            }
        }
        self.n += 1;
        Ok(())
    }

    /// Cierra el archivo (espera a que AVFoundation termine de escribirlo).
    pub fn cerrar(self) -> Result<()> {
        unsafe {
            self.entrada.markAsFinished();
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            let bloque = block2::RcBlock::new(move || {
                let _ = tx.send(());
            });
            self.escritor.finishWritingWithCompletionHandler(&bloque);
            rx.recv_timeout(Duration::from_secs(120)).map_err(|_| anyhow_tr!("AVAssetWriter no terminó de escribir", "AVAssetWriter did not finish writing"))?;
            if self.escritor.status() != AVAssetWriterStatus::Completed {
                let e = self.escritor.error().map(|e| e.localizedDescription().to_string()).unwrap_or_default(); bail_tr!("el MP4 quedó incompleto: {e}", "the MP4 is incomplete: {e}");
            }
        }
        Ok(())
    }
}

/// Ola PRO · lee un MP4 cuadro a cuadro (`AVAssetReader`, decodificación por hardware) en RGB de 8
/// bits: para EXTENDER un clip (su último cuadro es el primero del siguiente y los dos se unen).
pub struct Lector {
    lector: Retained<objc2_av_foundation::AVAssetReader>,
    salida: Retained<objc2_av_foundation::AVAssetReaderTrackOutput>,
    /// cuadros por segundo declarados por la pista
    pub cps: f32,
}

impl Lector {
    pub fn abrir(ruta: &Path) -> Result<Lector> {
        use objc2_av_foundation::{AVAssetReader, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVURLAsset};
        if !ruta.is_file() {
            bail_tr!("no existe el video {}", "the video {} does not exist", ruta.display());
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(&ruta.to_string_lossy()));
        unsafe {
            let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
            let medio = AVMediaTypeVideo.ok_or_else(|| anyhow_tr!("AVFoundation sin video", "AVFoundation without video"))?;
            // (la versión asíncrona `loadTracksWithMediaType` pide un bucle de eventos; esta basta en un hilo aparte)
            #[allow(deprecated)]
            let pistas = asset.tracksWithMediaType(medio);
            let pista = pistas.firstObject().ok_or_else(|| anyhow_tr!("{} no trae pista de video", "{} has no video track", ruta.display()))?;
            let lector = AVAssetReader::assetReaderWithAsset_error(&asset).map_err(|e| anyhow!("AVAssetReader: {}", e.localizedDescription()))?;
            // BGRA de 8 bits: lo mismo que se escribe, sin conversión de color en el camino
            let clave_formato: &NSString = &*(objc2_core_video::kCVPixelBufferPixelFormatTypeKey as *const objc2_core_foundation::CFString as *const NSString);
            let ajustes = NSDictionary::<NSString, AnyObject>::from_slices(&[clave_formato], &[&*NSNumber::new_u32(kCVPixelFormatType_32BGRA) as &AnyObject]);
            let salida = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&pista, Some(&ajustes));
            salida.setAlwaysCopiesSampleData(false);
            if !lector.canAddOutput(&salida) {
                bail_tr!("AVAssetReader no acepta la salida de video", "AVAssetReader does not accept the video output");
            }
            lector.addOutput(&salida);
            if !lector.startReading() || lector.status() == AVAssetReaderStatus::Failed {
                let e = lector.error().map(|e| e.localizedDescription().to_string()).unwrap_or_default();
                bail_tr!("no pude leer {}: {e}", "could not read {}: {e}", ruta.display());
            }
            let cps = pista.nominalFrameRate();
            Ok(Lector { lector, salida, cps })
        }
    }

    /// El cuadro siguiente: (ancho, alto, RGB), o `None` al terminar.
    pub fn siguiente(&mut self) -> Result<Option<(usize, usize, Vec<u8>)>> {
        use objc2_av_foundation::AVAssetReaderStatus;
        use objc2_core_video::{CVPixelBufferGetHeight, CVPixelBufferGetWidth};
        unsafe {
            loop {
                let Some(muestra) = self.salida.copyNextSampleBuffer() else {
                    if self.lector.status() == AVAssetReaderStatus::Failed {
                        let e = self.lector.error().map(|e| e.localizedDescription().to_string()).unwrap_or_default();
                        bail_tr!("el video se cortó al leerlo: {e}", "the video broke off while reading: {e}");
                    }
                    return Ok(None);
                };
                // las muestras «sólo marcador» no traen imagen: se saltan
                let Some(img) = muestra.image_buffer() else { continue };
                let (w, h) = (CVPixelBufferGetWidth(&img), CVPixelBufferGetHeight(&img));
                CVPixelBufferLockBaseAddress(&img, CVPixelBufferLockFlags::ReadOnly);
                let base = CVPixelBufferGetBaseAddress(&img) as *const u8;
                let fila = CVPixelBufferGetBytesPerRow(&img);
                let mut rgb = vec![0u8; w * h * 3];
                if !base.is_null() {
                    for y in 0..h {
                        let s = std::slice::from_raw_parts(base.add(y * fila), w * 4);
                        let d = &mut rgb[y * w * 3..(y + 1) * w * 3];
                        for x in 0..w {
                            d[x * 3] = s[x * 4 + 2];
                            d[x * 3 + 1] = s[x * 4 + 1];
                            d[x * 3 + 2] = s[x * 4];
                        }
                    }
                }
                CVPixelBufferUnlockBaseAddress(&img, CVPixelBufferLockFlags::ReadOnly);
                if base.is_null() {
                    bail_tr!("cuadro de video sin datos", "video frame without data");
                }
                return Ok(Some((w, h, rgb)));
            }
        }
    }
}

/// El último cuadro de un MP4 (ancho, alto, RGB) y cuántos cuadros tiene.
pub fn ultimo_cuadro(ruta: &Path) -> Result<(usize, usize, Vec<u8>, usize)> {
    let mut l = Lector::abrir(ruta)?;
    let mut ultimo = None;
    let mut n = 0;
    while let Some(c) = l.siguiente()? {
        ultimo = Some(c);
        n += 1;
    }
    let (w, h, rgb) = ultimo.ok_or_else(|| anyhow_tr!("{} no tiene cuadros", "{} has no frames", ruta.display()))?;
    Ok((w, h, rgb, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ola PRO: lo que se escribe se lee igual (cuadros, tamaño y, con la pérdida de H.264, colores
    /// cercanos): base de «extender clip».
    #[test]
    fn lee_lo_que_escribe() {
        let dir = std::env::temp_dir().join(format!("gotv-mp4-lee-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ruta = dir.join("ida.mp4");
        let (w, h) = (128usize, 96usize);
        let cuadro = |f: usize| -> Vec<u8> { (0..w * h).flat_map(|_| [(f * 20) as u8, 128, 255 - (f * 20) as u8]).collect() };
        let mut m = Mp4::nuevo(&ruta, w, h, 24).unwrap();
        for f in 0..10 {
            m.cuadro(&cuadro(f)).unwrap();
        }
        m.cerrar().unwrap();
        let mut l = Lector::abrir(&ruta).unwrap();
        assert!((l.cps - 24.0).abs() < 0.5, "{} cps", l.cps);
        let mut n = 0;
        while let Some((lw, lh, rgb)) = l.siguiente().unwrap() {
            assert_eq!((lw, lh, rgb.len()), (w, h, w * h * 3));
            let esperado = cuadro(n);
            let medio = (h / 2 * w + w / 2) * 3;
            for c in 0..3 {
                assert!((rgb[medio + c] as i32 - esperado[medio + c] as i32).abs() <= 12, "cuadro {n} canal {c}: {} vs {}", rgb[medio + c], esperado[medio + c]);
            }
            n += 1;
        }
        assert_eq!(n, 10);
        let (uw, uh, u, total) = ultimo_cuadro(&ruta).unwrap();
        assert_eq!((uw, uh, total), (w, h, 10));
        assert!((u[0] as i32 - 180).abs() <= 12, "el último es el cuadro 9: {}", u[0]);
        assert!(Lector::abrir(&dir.join("no-existe.mp4")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Un MP4 de 12 cuadros de 64×48 (un degradado que se mueve): se escribe, empieza con la caja
    /// `ftyp` de MPEG-4 y trae su `moov`. Sin modelos.
    #[test]
    fn escribe_un_mp4() {
        let dir = std::env::temp_dir().join(format!("gotv-mp4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ruta = dir.join("prueba.mp4");
        let (w, h) = (64usize, 48usize);
        let mut m = Mp4::nuevo(&ruta, w, h, 24).unwrap();
        for f in 0..12u8 {
            let rgb: Vec<u8> = (0..w * h).flat_map(|i| [((i % w) * 4) as u8, ((i / w) * 5) as u8, f * 20]).collect();
            m.cuadro(&rgb).unwrap();
        }
        m.cerrar().unwrap();
        let b = std::fs::read(&ruta).unwrap();
        assert_eq!(&b[4..8], b"ftyp", "cabecera MPEG-4");
        assert!(b.windows(4).any(|v| v == b"moov"), "sin índice moov");
        assert!(b.len() > 500, "MP4 de {} bytes", b.len());
        if let Ok(copia) = std::env::var("GOTV_MP4_GUARDAR") {
            std::fs::copy(&ruta, copia).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Diagnóstico (no corre solo): la diferencia media entre cuadros seguidos de un MP4
    /// (`GOTV_MP4_CONTINUIDAD=clip.mp4`): un salto en la unión de un clip extendido se ve como un pico.
    #[test]
    #[ignore]
    fn continuidad_de_un_clip() {
        let ruta = std::env::var("GOTV_MP4_CONTINUIDAD").expect("GOTV_MP4_CONTINUIDAD=clip.mp4");
        let mut l = Lector::abrir(Path::new(&ruta)).unwrap();
        let mut previo: Option<Vec<u8>> = None;
        let mut difs = Vec::new();
        while let Some((_, _, rgb)) = l.siguiente().unwrap() {
            if let Some(p) = &previo {
                difs.push(p.iter().zip(&rgb).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as u64).sum::<u64>() as f64 / rgb.len() as f64);
            }
            previo = Some(rgb);
        }
        for (i, d) in difs.iter().enumerate() {
            println!("{:3}→{:3}: {d:.2}", i, i + 1);
        }
    }

    #[test]
    fn rechaza_tamano_impar() {
        assert!(Mp4::nuevo(&std::env::temp_dir().join("x.mp4"), 63, 48, 24).is_err());
    }
}

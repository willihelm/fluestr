# Offline Intercom Transcription Desktop App

## Ziel

Baue eine Offline-Desktop-App für Windows und macOS, die mehrere Audioeingänge in Echtzeit transkribiert und die Ergebnisse in einer Chat-ähnlichen Ansicht anzeigt.

Primärer Use Case:
- Intercom-Streams für Audio-Techniker.
- Nahezu Live-Transkription.
- Bis zu 8 gleichzeitige Streams.
- Häufigster Zielrechner: Apple Silicon.

Nicht-Ziele für den ersten MVP:
- Keine Cloud-STT.
- Kein Audio-Routing-Mixer als Produktziel.
- Kein komplexes Speaker-Diarization-System im ersten Schritt.
- Keine kollaborative Multi-User-Sync-Plattform.

## Tech Stack

### Frontend
- Tauri WebView.
- React oder Svelte für die UI.
- TypeScript.

### Backend
- Rust.
- Tauri Commands für Ansteuerung.
- Tauri Events für Live-Updates.

### Transkription
- whisper.cpp lokal/offline.
- Apple Silicon: Metal aktivieren.
- Optional: CoreML-Encoder-Acceleration auf macOS.

### Audio
- Pro Stream eigener Capture-Channel.
- Interne Verarbeitung in 16 kHz mono PCM.
- Ring Buffer pro Stream.

### Persistence
- SQLite für Sessions, Segmente, Gerätezustände und Export.

## Zielarchitektur

```text
[Audio Input 1..8]
      |
      v
[Capture Layer]
      |
      v
[Per-Stream Ring Buffer]
      |
      v
[Per-Stream VAD]
      |
      v
[Segment Scheduler / Queue]
      |
      v
[Whisper Worker Pool]
      |
      v
[Result Aggregator]
      |
      v
[Tauri Events]
      |
      v
[Chat UI]
      |
      v
[SQLite]

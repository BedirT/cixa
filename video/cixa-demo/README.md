# Cixa demo video

This directory contains the editable Remotion source for Cixa's product walkthrough.
The dashboard images are real Cixa demo states, and every command shown is copied from
the supported Docker-first setup.

## Preview

```bash
npm install
npm run dev
```

Open the `CixaDemo` composition in Remotion Studio.

## Voiceover

The checked-in voice track was generated locally with Kokoro-82M using `af_heart`.
To regenerate it, install Python 3.12, `kokoro`, `soundfile`, and `espeak-ng`, then run:

```bash
npm run voiceover
```

The script reads `scripts/narration.json`, writes one deterministic M4A per scene,
and updates the generated timing file used by Remotion. Set `KOKORO_PYTHON` when the
packages live in another Python environment.

## Render

```bash
npm run lint
npm run render
```

The final video is written to `docs/assets/cixa-demo.mp4`.

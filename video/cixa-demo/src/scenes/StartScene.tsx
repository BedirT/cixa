import {interpolate, useCurrentFrame} from "remotion";
import {Background, BrowserFrame, SceneTitle, Terminal} from "../components";
import {clamp} from "../theme";

export const StartScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <Background>
      <div style={{position: "absolute", inset: "82px 92px 128px"}}>
        <SceneTitle step="01 · Owner side" title="Start Cixa locally." copy="One Docker stack gives you the broker, owner console, and isolated agent bridge." />
        <Terminal
          title="cixa · zsh"
          revealEvery={22}
          lines={[
            {text: "$ git clone https://github.com/BedirT/cixa.git", accent: true},
            {text: "$ cd cixa", accent: true},
            {text: "$ ./scripts/cixa-docker up", accent: true},
            {text: "✓ broker healthy   ✓ console ready", dim: true},
            {text: "$ ./scripts/cixa-docker dashboard-token", accent: true},
            {text: "Open http://127.0.0.1:8765", dim: true},
          ]}
          style={{position: "absolute", left: 0, bottom: 34, width: 900, height: 460}}
        />
        <BrowserFrame
          src="assets/dashboard-today.png"
          style={{
            position: "absolute",
            width: 760,
            height: 510,
            right: -20,
            bottom: -8,
            opacity: interpolate(frame, [58, 82], [0, 1], clamp),
            translate: `${interpolate(frame, [58, 82], [70, 0], clamp)}px 0`,
          }}
          imageStyle={{width: 760, height: "auto", translate: "0 -8px"}}
        />
      </div>
    </Background>
  );
};

import {interpolate, useCurrentFrame} from "remotion";
import {Background, BrowserFrame, Cursor, Pill, SceneTitle} from "../components";
import {clamp} from "../theme";

export const AgentScene: React.FC = () => {
  const frame = useCurrentFrame();
  const settingsOpacity = interpolate(frame, [205, 230], [0, 1], clamp);
  return (
    <Background>
      <div style={{position: "absolute", inset: "70px 86px 122px"}}>
        <SceneTitle step="02 · Capability" title="Give one agent one identity." copy="Start in Approval required, then make every limit explicit." />
        <div style={{position: "absolute", right: 0, top: 28}}>
          <Pill tone="gold">Secret stays in the private volume</Pill>
        </div>
        <BrowserFrame
          src="assets/dashboard-agents.png"
          style={{position: "absolute", left: 0, right: 0, bottom: 10, height: 690}}
          imageStyle={{width: "100%", height: "auto", translate: "0 -10px"}}
        >
          <Cursor x={675} y={355} clickAt={150} />
        </BrowserFrame>
        <BrowserFrame
          src="assets/dashboard-agent-settings.png"
          style={{
            position: "absolute",
            left: 0,
            right: 0,
            bottom: 10,
            height: 690,
            opacity: settingsOpacity,
          }}
          imageStyle={{width: "100%", height: "auto", translate: "0 -10px"}}
        >
          <div
            style={{
              position: "absolute",
              right: 55,
              top: 175,
              width: 410,
              height: 225,
              border: "4px solid rgba(213,167,78,.9)",
              borderRadius: 22,
              boxShadow: "0 0 0 999px rgba(13,23,37,.08)",
            }}
          />
        </BrowserFrame>
      </div>
    </Background>
  );
};

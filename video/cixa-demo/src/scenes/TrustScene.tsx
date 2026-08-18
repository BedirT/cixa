import {interpolate, useCurrentFrame} from "remotion";
import {Background, BrowserFrame, Pill, SceneTitle} from "../components";
import {clamp, colors} from "../theme";

const Boundary: React.FC<{number: string; title: string; copy: string; at: number}> = ({number, title, copy, at}) => {
  const frame = useCurrentFrame();
  return (
    <div
      style={{
        padding: "20px 22px",
        borderRadius: 19,
        background: "rgba(255,255,255,.86)",
        border: "1px solid rgba(83,111,133,.18)",
        opacity: interpolate(frame, [at, at + 16], [0, 1], clamp),
        translate: `0 ${interpolate(frame, [at, at + 16], [18, 0], clamp)}px`,
      }}
    >
      <div style={{display: "flex", alignItems: "center", gap: 13}}>
        <span style={{display: "grid", placeItems: "center", width: 32, height: 32, borderRadius: 20, background: colors.navy, color: colors.white, fontSize: 15, fontWeight: 800}}>{number}</span>
        <strong style={{fontSize: 20, color: colors.ink}}>{title}</strong>
      </div>
      <p style={{margin: "10px 0 0 45px", color: colors.muted, fontSize: 17, lineHeight: 1.4}}>{copy}</p>
    </div>
  );
};

export const TrustScene: React.FC = () => (
  <Background>
    <div style={{position: "absolute", inset: "70px 86px 122px"}}>
      <SceneTitle step="06 · Payment boundary" title="Arm the sensitive side deliberately." copy="KOHO remains owner-controlled. Automation starts only inside reviewed boundaries." />
      <BrowserFrame
        src="assets/dashboard-trust.png"
        style={{position: "absolute", left: 0, bottom: 10, width: 1160, height: 690}}
        imageStyle={{width: 1160, height: "auto", translate: "0 -10px"}}
      >
        <div style={{position: "absolute", right: 32, top: 40}}><Pill tone="gold">Trust</Pill></div>
      </BrowserFrame>
      <div style={{position: "absolute", right: 0, width: 565, bottom: 20, display: "grid", gap: 14}}>
        <Boundary number="1" title="Reference, not credentials" copy="Cixa stores masked KOHO setup metadata." at={54} />
        <Boundary number="2" title="Profile the merchant" copy="Pin origins, hosted fields, selectors, and evidence." at={90} />
        <Boundary number="3" title="Open a short card session" copy="The helper expires by time and checkout count." at={126} />
        <Boundary number="4" title="Card stays out of state" copy="Never exposed to the agent, logs, or receipts." at={162} />
      </div>
    </div>
  </Background>
);

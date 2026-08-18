import {Img, interpolate, staticFile, useCurrentFrame} from "remotion";
import {Background, Brand, Pill} from "../components";
import {clamp, colors} from "../theme";

export const OutroScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <Background dark>
      <div style={{position: "absolute", inset: "100px 118px 140px", display: "grid", gridTemplateColumns: "1fr 1.12fr", alignItems: "center", gap: 80}}>
        <div style={{opacity: interpolate(frame, [0, 20], [0, 1], clamp), translate: `${interpolate(frame, [0, 20], [-22, 0], clamp)}px 0`}}>
          <Brand dark compact />
          <h1 style={{fontFamily: "Newsreader", fontWeight: 450, fontSize: 76, lineHeight: 1.03, letterSpacing: -2.5, color: colors.cream, margin: "38px 0 24px"}}>
            The agent shops.<br />Cixa holds the boundary.
          </h1>
          <p style={{fontSize: 27, lineHeight: 1.5, color: "#afc0d1", margin: "0 0 32px"}}>
            Start small in Approval required. Expand authority only after the complete flow earns it.
          </p>
          <div style={{display: "flex", gap: 12}}>
            <Pill>Docker first</Pill>
            <Pill tone="gold">Local first</Pill>
            <Pill tone="green">AGPLv3</Pill>
          </div>
        </div>
        <div
          style={{
            height: 690,
            borderRadius: 26,
            overflow: "hidden",
            background: "rgba(255,255,255,.96)",
            boxShadow: "0 38px 120px rgba(0,0,0,.42)",
            opacity: interpolate(frame, [25, 48], [0, 1], clamp),
            scale: interpolate(frame, [25, 48], [0.95, 1], clamp),
          }}
        >
          <Img src={staticFile("assets/cixa-architecture.svg")} style={{width: "100%", height: "100%", objectFit: "cover", objectPosition: "center top"}} />
        </div>
      </div>
    </Background>
  );
};

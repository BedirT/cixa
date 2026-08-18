import {Easing, interpolate, useCurrentFrame} from "remotion";
import {Background, Brand, Pill} from "../components";
import {clamp, colors} from "../theme";

export const IntroScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <Background dark>
      <div style={{position: "absolute", inset: "120px 130px 150px", display: "flex", alignItems: "center"}}>
        <div
          style={{
            width: "100%",
            opacity: interpolate(frame, [0, 24], [0, 1], {
              ...clamp,
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            }),
            scale: interpolate(frame, [0, 30], [0.94, 1], {
              ...clamp,
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            }),
          }}
        >
          <Brand dark />
          <h1
            style={{
              margin: "48px 0 18px",
              maxWidth: 1260,
              fontFamily: "Newsreader",
              fontWeight: 450,
              fontSize: 92,
              lineHeight: 1.02,
              letterSpacing: -3,
              color: colors.cream,
            }}
          >
            Real payments, with a boundary your agent cannot talk around.
          </h1>
          <p style={{fontSize: 31, color: "#aebfd0", margin: "0 0 42px"}}>
            Docker-first checkout orchestration for software agents.
          </p>
          <div style={{display: "flex", gap: 14}}>
            <Pill>Agent shops</Pill>
            <Pill tone="gold">Cixa decides</Pill>
            <Pill tone="green">Owner stays in control</Pill>
          </div>
        </div>
      </div>
    </Background>
  );
};

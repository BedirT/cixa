import {interpolate, useCurrentFrame} from "remotion";
import {Background, Pill, SceneTitle} from "../components";
import {clamp, colors, shadows} from "../theme";

const FlowCard: React.FC<{at: number; number: string; title: string; copy: string; tone?: "blue" | "gold" | "green"}> = ({at, number, title, copy, tone = "blue"}) => {
  const frame = useCurrentFrame();
  return (
    <div
      style={{
        flex: 1,
        minHeight: 220,
        padding: 26,
        borderRadius: 24,
        background: "rgba(255,255,255,.9)",
        border: "1px solid rgba(77,106,129,.18)",
        boxShadow: shadows.soft,
        opacity: interpolate(frame, [at, at + 18], [0, 1], clamp),
        translate: `0 ${interpolate(frame, [at, at + 18], [28, 0], clamp)}px`,
      }}
    >
      <Pill tone={tone}>{number}</Pill>
      <h3 style={{fontFamily: "Newsreader", fontSize: 36, margin: "20px 0 10px", color: colors.ink}}>{title}</h3>
      <p style={{fontSize: 19, lineHeight: 1.45, color: colors.muted, margin: 0}}>{copy}</p>
    </div>
  );
};

export const CheckoutScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <Background dark>
      <div style={{position: "absolute", inset: "70px 90px 126px"}}>
        <SceneTitle dark step="07 · Execute and reconcile" title="Submit once. Never guess the outcome." copy="Cixa persists the decision before it touches checkout, then waits for provider evidence." />
        <div style={{position: "absolute", left: 0, right: 0, bottom: 170, display: "flex", gap: 20}}>
          <FlowCard at={40} number="01" title="Reserve" copy="Policy and ledger reserve the exact amount before execution." />
          <FlowCard at={78} number="02" title="Recheck" copy="The isolated browser confirms live merchant and checkout facts." tone="gold" />
          <FlowCard at={116} number="03" title="Submit once" copy="A timeout or ambiguous response is quarantined, not retried." />
          <FlowCard at={154} number="04" title="Reconcile" copy="The owner checks KOHO and records paid or declined." tone="green" />
        </div>
        <div
          style={{
            position: "absolute",
            left: 132,
            right: 132,
            bottom: 107,
            height: 5,
            borderRadius: 10,
            background: "rgba(255,255,255,.12)",
            overflow: "hidden",
          }}
        >
          <div
            style={{
              width: `${interpolate(frame, [45, 205], [0, 100], clamp)}%`,
              height: "100%",
              background: `linear-gradient(90deg, ${colors.blue}, ${colors.gold}, ${colors.green})`,
            }}
          />
        </div>
        <div style={{position: "absolute", right: 0, bottom: 38, opacity: interpolate(frame, [205, 225], [0, 1], clamp)}}>
          <Pill tone="green">✓ Provider outcome recorded</Pill>
        </div>
      </div>
    </Background>
  );
};

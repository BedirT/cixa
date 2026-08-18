import {interpolate, useCurrentFrame} from "remotion";
import {Background, Pill, SceneTitle} from "../components";
import {clamp, colors, shadows} from "../theme";

const ToolCall: React.FC<{name: string; result: string; at: number}> = ({name, result, at}) => {
  const frame = useCurrentFrame();
  return (
    <div
      style={{
        padding: "17px 20px",
        borderRadius: 15,
        border: "1px solid rgba(74,108,133,.2)",
        background: "rgba(255,255,255,.75)",
        opacity: interpolate(frame, [at, at + 14], [0, 1], clamp),
        translate: `${interpolate(frame, [at, at + 14], [18, 0], clamp)}px 0`,
      }}
    >
      <div style={{fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace", fontSize: 18, fontWeight: 760, color: colors.blue}}>↳ {name}</div>
      <div style={{fontSize: 18, color: colors.muted, marginTop: 6}}>{result}</div>
    </div>
  );
};

export const PreflightScene: React.FC = () => (
  <Background>
    <div style={{position: "absolute", inset: "72px 100px 126px"}}>
      <SceneTitle step="04 · Connection check" title="Ask before you buy." copy="A safe preflight proves the agent can see only its own authority." />
      <div
        style={{
          position: "absolute",
          right: 0,
          bottom: 12,
          width: 1110,
          minHeight: 610,
          padding: 34,
          borderRadius: 26,
          background: "rgba(255,255,255,.8)",
          border: "1px solid rgba(74,108,133,.18)",
          boxShadow: shadows.soft,
        }}
      >
        <div style={{display: "flex", justifyContent: "space-between", alignItems: "center"}}>
          <Pill>Fresh agent session</Pill>
          <span style={{fontSize: 17, color: colors.muted}}>Research Runner · Approval required</span>
        </div>
        <div style={{marginTop: 24, padding: "20px 24px", borderRadius: 18, background: colors.navy, color: colors.white, fontSize: 22, lineHeight: 1.45}}>
          Check your Cixa status, capabilities, and remaining budget. Do not make a purchase.
        </div>
        <div style={{display: "grid", gap: 12, marginTop: 18}}>
          <ToolCall name="cixa_get_status" result="Watching · owner controls available" at={42} />
          <ToolCall name="cixa_get_capabilities" result="create_intent · execute_intent · receive_instructions" at={78} />
          <ToolCall name="cixa_get_budget" result="CA$132.00 remaining · CA$60.00 per purchase" at={114} />
        </div>
        <p style={{margin: "20px 4px 0", fontSize: 22, color: colors.ink}}>Connected. I can report my limits without requesting payment credentials.</p>
      </div>
      <div style={{position: "absolute", left: 0, bottom: 195, width: 410, padding: 28, borderRadius: 24, background: "#f9efd9", color: "#77571f", fontSize: 22, lineHeight: 1.5}}>
        <strong style={{display: "block", marginBottom: 8}}>A useful first test</strong>
        No purchase, no card session, and no owner token.
      </div>
    </div>
  </Background>
);

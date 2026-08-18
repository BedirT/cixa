import {interpolate, useCurrentFrame} from "remotion";
import {Background, BrowserFrame, Cursor, Pill, SceneTitle} from "../components";
import {clamp, colors, shadows} from "../theme";

export const TodayScene: React.FC = () => {
  const frame = useCurrentFrame();
  const dialog = interpolate(frame, [240, 260], [0, 1], clamp);
  return (
    <Background>
      <div style={{position: "absolute", inset: "68px 82px 122px"}}>
        <SceneTitle step="05 · Owner decision" title="See the exact purchase." copy="The decision binds the agent, merchant, amount, items, and checkout facts together." />
        <div style={{position: "absolute", right: 0, top: 28, display: "flex", gap: 10}}>
          <Pill tone="gold">Waiting: 3</Pill>
          <Pill tone="green">Still allowed: CA$226</Pill>
        </div>
        <BrowserFrame
          src="assets/dashboard-today.png"
          style={{position: "absolute", left: 0, right: 0, bottom: 6, height: 690}}
          imageStyle={{width: "100%", height: "auto", translate: "0 -8px"}}
        >
          <Cursor x={420} y={409} clickAt={155} />
          <div
            style={{
              position: "absolute",
              inset: 0,
              background: `rgba(17,31,47,${dialog * 0.28})`,
              pointerEvents: "none",
            }}
          />
          <div
            style={{
              position: "absolute",
              left: "50%",
              top: "50%",
              width: 640,
              padding: 30,
              borderRadius: 24,
              background: "rgba(255,255,255,.96)",
              boxShadow: shadows.soft,
              opacity: dialog,
              translate: `-50% -${46 + (1 - dialog) * 10}%`,
            }}
          >
            <div style={{fontSize: 15, textTransform: "uppercase", letterSpacing: 2, color: colors.gold, fontWeight: 800}}>Owner confirmation</div>
            <h2 style={{fontFamily: "Newsreader", fontSize: 39, margin: "10px 0 8px", color: colors.ink}}>Allow this one purchase?</h2>
            <p style={{fontSize: 19, lineHeight: 1.4, color: colors.muted, margin: "0 0 20px"}}>This approval applies once. It does not permanently trust the merchant.</p>
            <div style={{display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10, marginBottom: 22}}>
              {[["Amount", "CA$32.00"], ["Merchant", "open-data.example"], ["Agent", "Research Runner"]].map(([label, value]) => (
                <div key={label} style={{padding: 14, borderRadius: 14, background: "#f2f6f8"}}>
                  <div style={{fontSize: 13, color: colors.muted}}>{label}</div>
                  <div style={{fontSize: 16, color: colors.ink, fontWeight: 720, marginTop: 5}}>{value}</div>
                </div>
              ))}
            </div>
            <div style={{display: "flex", gap: 12}}>
              <div style={{padding: "13px 20px", borderRadius: 13, background: colors.navy, color: colors.white, fontWeight: 750}}>Allow once</div>
              <div style={{padding: "13px 20px", borderRadius: 13, border: "1px solid #d9e0e5", color: colors.red, fontWeight: 750}}>Decline</div>
            </div>
          </div>
        </BrowserFrame>
      </div>
    </Background>
  );
};

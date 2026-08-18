import {interpolate, useCurrentFrame} from "remotion";
import {Background, Pill, SceneTitle, Terminal} from "../components";
import {clamp, colors, shadows} from "../theme";

export const ConnectScene: React.FC = () => {
  const frame = useCurrentFrame();
  const boundary = [
    ["No network", "The MCP bridge cannot shop or browse."],
    ["Read-only IPC", "It receives one capability filename."],
    ["No owner state", "Card data and dashboard secrets stay outside."],
  ];
  return (
    <Background dark>
      <div style={{position: "absolute", inset: "76px 92px 128px"}}>
        <SceneTitle dark step="03 · Agent setup" title="Install guidance. Connect the bridge." copy="The skill explains safe behavior. The capability and broker enforce authority." />
        <div style={{display: "grid", gridTemplateColumns: "1.16fr .84fr", gap: 30, position: "absolute", left: 0, right: 0, bottom: 14}}>
          <Terminal
            title="agent project · setup"
            revealEvery={24}
            lines={[
              {text: "$ ./scripts/install-agent-skill --target all", accent: true},
              {text: "✓ Codex skill installed", dim: true},
              {text: "✓ Claude Code skill installed", dim: true},
              {text: "$ ./scripts/cixa-docker agent-config \\", accent: true},
              {text: "    research-runner.token", accent: true},
              {text: "✓ MCP configuration generated", dim: true},
            ]}
            style={{height: 490}}
          />
          <div style={{display: "flex", flexDirection: "column", gap: 18}}>
            {boundary.map(([title, copy], index) => (
              <div
                key={title}
                style={{
                  flex: 1,
                  padding: "24px 26px",
                  borderRadius: 20,
                  background: "rgba(255,255,255,.07)",
                  border: "1px solid rgba(255,255,255,.1)",
                  boxShadow: shadows.dark,
                  opacity: interpolate(frame, [70 + index * 24, 88 + index * 24], [0, 1], clamp),
                  translate: `${interpolate(frame, [70 + index * 24, 88 + index * 24], [24, 0], clamp)}px 0`,
                }}
              >
                <Pill tone={index === 1 ? "gold" : "blue"}>{title}</Pill>
                <p style={{margin: "16px 0 0", fontSize: 21, lineHeight: 1.4, color: colors.white}}>{copy}</p>
              </div>
            ))}
          </div>
        </div>
      </div>
    </Background>
  );
};

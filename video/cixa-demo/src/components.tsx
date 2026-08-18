import type {CSSProperties, ReactNode} from "react";
import {
  AbsoluteFill,
  Easing,
  Img,
  interpolate,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import {clamp, colors, shadows} from "./theme";

export const Background: React.FC<{dark?: boolean; children: ReactNode}> = ({
  dark = false,
  children,
}) => {
  const frame = useCurrentFrame();
  return (
    <AbsoluteFill
      style={{
        overflow: "hidden",
        background: dark
          ? "linear-gradient(135deg, #0b1420 0%, #142337 55%, #0d1826 100%)"
          : "linear-gradient(135deg, #edf8fc 0%, #f8fbfc 48%, #f8f2e8 100%)",
        color: dark ? colors.white : colors.ink,
        fontFamily: "Manrope",
      }}
    >
      <div
        style={{
          position: "absolute",
          width: 980,
          height: 980,
          borderRadius: "50%",
          left: -330,
          top: -430,
          background: dark
            ? "radial-gradient(circle, rgba(70,141,174,.18), transparent 68%)"
            : "radial-gradient(circle, rgba(91,175,211,.25), transparent 68%)",
          translate: `${interpolate(frame, [0, 900], [0, 45], clamp)}px ${interpolate(
            frame,
            [0, 900],
            [0, 24],
            clamp,
          )}px`,
        }}
      />
      <div
        style={{
          position: "absolute",
          width: 900,
          height: 900,
          borderRadius: "50%",
          right: -240,
          bottom: -460,
          background: dark
            ? "radial-gradient(circle, rgba(213,167,78,.13), transparent 67%)"
            : "radial-gradient(circle, rgba(213,167,78,.19), transparent 67%)",
        }}
      />
      {children}
    </AbsoluteFill>
  );
};

export const Brand: React.FC<{dark?: boolean; compact?: boolean}> = ({
  dark = false,
  compact = false,
}) => (
  <div style={{display: "flex", alignItems: "center", gap: compact ? 16 : 28}}>
    <Img
      src={staticFile("assets/cixa-mark.svg")}
      style={{
        width: compact ? 48 : 92,
        height: compact ? 48 : 92,
        filter: dark
          ? "brightness(0) saturate(100%) invert(77%) sepia(48%) saturate(735%) hue-rotate(357deg) brightness(91%) contrast(88%)"
          : undefined,
      }}
    />
    <span
      style={{
        fontFamily: "Newsreader",
        fontSize: compact ? 48 : 96,
        lineHeight: 1,
        color: dark ? colors.cream : colors.ink,
      }}
    >
      Cixa
    </span>
  </div>
);

export const SceneTitle: React.FC<{
  step: string;
  title: string;
  copy?: string;
  dark?: boolean;
}> = ({step, title, copy, dark = false}) => {
  const frame = useCurrentFrame();
  return (
    <div
      style={{
        opacity: interpolate(frame, [0, 18], [0, 1], {
          ...clamp,
          easing: Easing.bezier(0.16, 1, 0.3, 1),
        }),
        translate: `0 ${interpolate(frame, [0, 18], [24, 0], {
          ...clamp,
          easing: Easing.bezier(0.16, 1, 0.3, 1),
        })}px`,
      }}
    >
      <div
        style={{
          color: colors.gold,
          fontSize: 20,
          letterSpacing: 3.4,
          fontWeight: 800,
          textTransform: "uppercase",
          marginBottom: 18,
        }}
      >
        {step}
      </div>
      <h1
        style={{
          margin: 0,
          maxWidth: 980,
          color: dark ? colors.cream : colors.ink,
          fontFamily: "Newsreader",
          fontSize: 78,
          lineHeight: 0.98,
          fontWeight: 500,
          letterSpacing: -2.5,
        }}
      >
        {title}
      </h1>
      {copy ? (
        <p
          style={{
            margin: "24px 0 0",
            maxWidth: 900,
            color: dark ? "#b9c8d8" : colors.muted,
            fontSize: 27,
            lineHeight: 1.42,
          }}
        >
          {copy}
        </p>
      ) : null}
    </div>
  );
};

export const BrowserFrame: React.FC<{
  src: string;
  style?: CSSProperties;
  imageStyle?: CSSProperties;
  address?: string;
  children?: ReactNode;
}> = ({src, style, imageStyle, address = "127.0.0.1:8765", children}) => (
  <div
    style={{
      overflow: "hidden",
      borderRadius: 24,
      border: "1px solid rgba(94, 119, 141, .24)",
      background: "rgba(255,255,255,.84)",
      boxShadow: shadows.soft,
      ...style,
    }}
  >
    <div
      style={{
        height: 52,
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "0 20px",
        background: "rgba(247,249,251,.96)",
        borderBottom: "1px solid rgba(94, 119, 141, .16)",
      }}
    >
      {["#ff6d68", "#efbf4a", "#58c26e"].map((color) => (
        <span key={color} style={{width: 12, height: 12, borderRadius: 20, background: color}} />
      ))}
      <div
        style={{
          marginLeft: 18,
          height: 30,
          minWidth: 390,
          padding: "5px 18px",
          borderRadius: 10,
          background: "#edf1f4",
          color: "#778392",
          fontSize: 14,
          textAlign: "center",
        }}
      >
        {address}
      </div>
    </div>
    <div style={{position: "relative", height: "calc(100% - 52px)", overflow: "hidden"}}>
      <Img src={staticFile(src)} style={{display: "block", ...imageStyle}} />
      {children}
    </div>
  </div>
);

export const Terminal: React.FC<{
  lines: Array<{text: string; accent?: boolean; dim?: boolean}>;
  title?: string;
  style?: CSSProperties;
  revealEvery?: number;
}> = ({lines, title = "Terminal", style, revealEvery = 20}) => {
  const frame = useCurrentFrame();
  return (
    <div
      style={{
        borderRadius: 22,
        overflow: "hidden",
        background: "rgba(8, 16, 26, .97)",
        color: "#e7edf3",
        border: "1px solid rgba(255,255,255,.11)",
        boxShadow: shadows.dark,
        ...style,
      }}
    >
      <div
        style={{
          height: 52,
          display: "flex",
          alignItems: "center",
          padding: "0 20px",
          borderBottom: "1px solid rgba(255,255,255,.08)",
          color: "#8fa2b5",
          fontSize: 14,
        }}
      >
        <span style={{letterSpacing: 1}}>{title}</span>
      </div>
      <div style={{padding: "26px 30px", fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace"}}>
        {lines.map((line, index) => (
          <div
            key={`${line.text}-${index}`}
            style={{
              minHeight: 36,
              opacity: interpolate(frame, [index * revealEvery, index * revealEvery + 9], [0, line.dim ? 0.52 : 1], clamp),
              translate: `${interpolate(frame, [index * revealEvery, index * revealEvery + 9], [-8, 0], clamp)}px 0`,
              color: line.accent ? "#8dd6f2" : undefined,
              fontSize: 20,
              lineHeight: 1.5,
              whiteSpace: "pre-wrap",
            }}
          >
            {line.text}
          </div>
        ))}
      </div>
    </div>
  );
};

export const Cursor: React.FC<{x: number; y: number; clickAt?: number}> = ({
  x,
  y,
  clickAt = 72,
}) => {
  const frame = useCurrentFrame();
  const clicked = interpolate(frame, [clickAt - 4, clickAt, clickAt + 8], [0, 1, 0], clamp);
  return (
    <div style={{position: "absolute", left: x, top: y, zIndex: 10}}>
      <div
        style={{
          position: "absolute",
          width: 52,
          height: 52,
          borderRadius: "50%",
          border: `3px solid rgba(45,130,167,${clicked * 0.65})`,
          scale: 0.7 + clicked * 0.65,
          left: -19,
          top: -19,
        }}
      />
      <svg width="30" height="38" viewBox="0 0 30 38" aria-hidden="true">
        <path d="M3 2L26 22L16 23L21 34L15 37L10 25L3 31Z" fill="#fff" stroke="#172131" strokeWidth="2.5" />
      </svg>
    </div>
  );
};

type CaptionWord = {text: string; start: number; end: number};

export const Captions: React.FC<{words: readonly CaptionWord[]}> = ({words}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const seconds = frame / fps;
  const foundIndex = words.findIndex((word) => word.end >= seconds);
  const activeIndex = foundIndex === -1 ? words.length - 1 : foundIndex;
  const pageStart = Math.floor(activeIndex / 9) * 9;
  const page = words.slice(pageStart, pageStart + 9);
  return (
    <div
      style={{
        position: "absolute",
        zIndex: 40,
        left: "50%",
        bottom: 34,
        translate: "-50% 0",
        maxWidth: 1280,
        minHeight: 64,
        padding: "14px 25px",
        borderRadius: 18,
        background: "rgba(7, 15, 25, .88)",
        border: "1px solid rgba(255,255,255,.13)",
        boxShadow: "0 16px 48px rgba(0,0,0,.2)",
        color: "#f8fbfd",
        fontSize: 25,
        fontWeight: 650,
        lineHeight: 1.35,
        textAlign: "center",
      }}
    >
      {page.map((word, index) => (
        <span
          key={`${word.start}-${word.text}`}
          style={{color: seconds >= word.start && seconds < word.end ? "#f0c76f" : undefined}}
        >
          {index ? " " : ""}
          {word.text}
        </span>
      ))}
    </div>
  );
};

export const Pill: React.FC<{children: ReactNode; tone?: "blue" | "gold" | "green"}> = ({
  children,
  tone = "blue",
}) => {
  const palette = {
    blue: ["#e3f2f8", colors.blue],
    gold: ["#fbf1dc", "#987127"],
    green: ["#e4f3eb", colors.green],
  }[tone];
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        padding: "10px 16px",
        borderRadius: 100,
        background: palette[0],
        color: palette[1],
        fontSize: 18,
        fontWeight: 750,
      }}
    >
      {children}
    </span>
  );
};

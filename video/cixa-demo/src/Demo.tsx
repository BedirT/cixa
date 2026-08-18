import {Fragment, type ComponentType} from "react";
import {Audio} from "@remotion/media";
import {TransitionSeries, linearTiming} from "@remotion/transitions";
import {fade} from "@remotion/transitions/fade";
import {AbsoluteFill, staticFile} from "remotion";
import {Captions} from "./components";
import {voiceover} from "./generated/voiceover";
import {AgentScene} from "./scenes/AgentScene";
import {CheckoutScene} from "./scenes/CheckoutScene";
import {ConnectScene} from "./scenes/ConnectScene";
import {IntroScene} from "./scenes/IntroScene";
import {OutroScene} from "./scenes/OutroScene";
import {PreflightScene} from "./scenes/PreflightScene";
import {StartScene} from "./scenes/StartScene";
import {TodayScene} from "./scenes/TodayScene";
import {TrustScene} from "./scenes/TrustScene";

export const FPS = 30;
export const TRANSITION_FRAMES = 15;

const sceneComponents: Record<(typeof voiceover)[number]["id"], ComponentType> = {
  intro: IntroScene,
  start: StartScene,
  agent: AgentScene,
  connect: ConnectScene,
  preflight: PreflightScene,
  today: TodayScene,
  trust: TrustScene,
  checkout: CheckoutScene,
  outro: OutroScene,
};

export const sceneFrames = voiceover.map((scene) => Math.ceil(scene.duration * FPS));
export const totalFrames =
  sceneFrames.reduce((total, duration) => total + duration, 0) -
  TRANSITION_FRAMES * (voiceover.length - 1);

const NarratedScene: React.FC<{
  scene: (typeof voiceover)[number];
  component: ComponentType;
}> = ({scene, component: Scene}) => (
  <AbsoluteFill>
    <Scene />
    <Audio src={staticFile(scene.audio)} volume={1} />
    <Captions words={scene.words} />
  </AbsoluteFill>
);

export const CixaDemo: React.FC = () => (
  <AbsoluteFill>
    <Audio src={staticFile("voiceover/ambient.m4a")} volume={0.55} />
    <TransitionSeries>
      {voiceover.map((scene, index) => {
        const Scene = sceneComponents[scene.id];
        return (
          <Fragment key={scene.id}>
            <TransitionSeries.Sequence
              durationInFrames={sceneFrames[index]}
              name={`${String(index + 1).padStart(2, "0")} · ${scene.id}`}
            >
              <NarratedScene scene={scene} component={Scene} />
            </TransitionSeries.Sequence>
            {index < voiceover.length - 1 ? (
              <TransitionSeries.Transition
                presentation={fade()}
                timing={linearTiming({durationInFrames: TRANSITION_FRAMES})}
              />
            ) : null}
          </Fragment>
        );
      })}
    </TransitionSeries>
  </AbsoluteFill>
);

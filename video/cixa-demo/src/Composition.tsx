import {loadFont} from "@remotion/fonts";
import {Composition, staticFile} from "remotion";
import {CixaDemo, FPS, totalFrames} from "./Demo";

void loadFont({
  family: "Manrope",
  url: staticFile("assets/manrope-latin.woff2"),
  weight: "200 800",
});

void loadFont({
  family: "Newsreader",
  url: staticFile("assets/newsreader-latin.woff2"),
  weight: "300 700",
});

export const DemoComposition = () => {
  return (
    <Composition
      id="CixaDemo"
      component={CixaDemo}
      durationInFrames={totalFrames}
      fps={FPS}
      width={1920}
      height={1080}
    />
  );
};

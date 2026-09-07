import React from "react";
import { NodeViewWrapper, type NodeViewProps } from "@tiptap/react";
import "./video-node.scss";
import type { VideoOptions } from "./video-extension.ts";

interface VideoAttrs {
  src: string;
  width?: string;
  height?: string;
  controls?: boolean;
  autoplay?: boolean;
  loop?: boolean;
  muted?: boolean;
  poster?: string;
}

export const VideoComponent: React.FC<NodeViewProps> = ({
  node,
  selected,
  extension,
}) => {
  const { src, width, height, controls, autoplay, loop, muted, poster } =
    node.attrs as VideoAttrs;

  if (!src) {
    return null;
  }

  const formatUrl = (extension.options as VideoOptions).formatUrl;
  const resolvedSrc = formatUrl ? formatUrl(src) : src;

  return (
    <NodeViewWrapper
      className={`video-node-view ${selected ? "selected" : ""}`}
    >
      <div className="video-wrapper">
        <video
          src={resolvedSrc}
          width={width}
          height={height}
          controls={controls}
          autoPlay={autoplay}
          loop={loop}
          muted={muted}
          poster={poster}
          className="video-element"
          preload="metadata"
        >
          Your browser does not support the video tag.
        </video>
      </div>
    </NodeViewWrapper>
  );
};

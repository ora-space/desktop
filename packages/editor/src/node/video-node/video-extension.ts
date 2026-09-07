import { Node, mergeAttributes } from "@tiptap/core";
import { ReactNodeViewRenderer } from "@tiptap/react";
import { VideoComponent } from "./video-component.tsx";

export type VideoAttributes = {
  src: string;
  width?: string;
  height?: string;
  controls?: boolean;
  autoplay?: boolean;
  loop?: boolean;
  muted?: boolean;
  poster?: string;
};

export interface VideoOptions {
  HTMLAttributes: Record<string, unknown>;
  /** Whether to include controls by default. @default true */
  controls: boolean;
  /** Whether to autoplay by default. @default false */
  autoplay: boolean;
  /** Whether to loop by default. @default false */
  loop: boolean;
  /** Whether to mute by default. @default false */
  muted: boolean;
  /** Optional URL formatter applied to video src before rendering (e.g. for local media servers). */
  formatUrl?: (url: string) => string;
}

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    video: {
      /**
       * Insert a video
       */
      setVideo: (options: VideoAttributes) => ReturnType;
      /**
       * Update video attributes
       */
      updateVideo: (options: Partial<VideoAttributes>) => ReturnType;
    };
  }
}

export const VideoExtension = Node.create<VideoOptions>({
  name: "video",

  addOptions() {
    return {
      HTMLAttributes: {},
      controls: true,
      autoplay: false,
      loop: false,
      muted: false,
      formatUrl: undefined,
    };
  },

  group: "block",

  draggable: true,

  atom: true,

  addAttributes() {
    return {
      src: {
        default: null,
        parseHTML: (element) => element.getAttribute("src"),
        renderHTML: (attributes) => {
          if (!attributes.src) {
            return {};
          }
          return {
            src: attributes.src,
          };
        },
      },
      width: {
        default: "100%",
        parseHTML: (element) => element.getAttribute("width"),
        renderHTML: (attributes) => {
          return {
            width: attributes.width,
          };
        },
      },
      height: {
        default: "auto",
        parseHTML: (element) => element.getAttribute("height"),
        renderHTML: (attributes) => {
          return {
            height: attributes.height,
          };
        },
      },
      controls: {
        default: this.options.controls,
        parseHTML: (element) => element.hasAttribute("controls"),
        renderHTML: (attributes) => {
          if (attributes.controls) {
            return {
              controls: "true",
            };
          }
          return {};
        },
      },
      autoplay: {
        default: this.options.autoplay,
        parseHTML: (element) => element.hasAttribute("autoplay"),
        renderHTML: (attributes) => {
          if (attributes.autoplay) {
            return {
              autoplay: "true",
            };
          }
          return {};
        },
      },
      loop: {
        default: this.options.loop,
        parseHTML: (element) => element.hasAttribute("loop"),
        renderHTML: (attributes) => {
          if (attributes.loop) {
            return {
              loop: "true",
            };
          }
          return {};
        },
      },
      muted: {
        default: this.options.muted,
        parseHTML: (element) => element.hasAttribute("muted"),
        renderHTML: (attributes) => {
          if (attributes.muted) {
            return {
              muted: "true",
            };
          }
          return {};
        },
      },
      poster: {
        default: null,
        parseHTML: (element) => element.getAttribute("poster"),
        renderHTML: (attributes) => {
          if (!attributes.poster) {
            return {};
          }
          return {
            poster: attributes.poster,
          };
        },
      },
    };
  },

  parseHTML() {
    return [
      {
        tag: "video[src]",
      },
    ];
  },

  renderHTML({ HTMLAttributes }) {
    return [
      "video",
      mergeAttributes(this.options.HTMLAttributes, HTMLAttributes),
    ];
  },

  addNodeView() {
    return ReactNodeViewRenderer(VideoComponent);
  },

  addCommands() {
    return {
      setVideo:
        (options: VideoAttributes) =>
        ({ commands }) => {
          return commands.insertContent({
            type: this.name,
            attrs: options,
          });
        },
      updateVideo:
        (options: Partial<VideoAttributes>) =>
        ({ commands }) => {
          return commands.updateAttributes(this.name, options);
        },
    };
  },
});

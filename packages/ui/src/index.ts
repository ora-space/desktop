/**
 * @ora/ui — Untitled UI component kit adapted for the Ora AI IDE.
 *
 * Values-only barrel (no type re-exports) to avoid collisions between
 * same-named local symbols (e.g. `styles`, `CommonProps`) in component files.
 * Import types from their deep paths when needed, e.g.
 *   import { type AvatarProps } from "@ora/ui/components/base/avatar/avatar";
 */

// buttons
export { Button } from "./components/base/buttons/button";
export { ButtonUtility } from "./components/base/buttons/button-utility";
export { CloseButton } from "./components/base/buttons/close-button";
export { ButtonGroup, ButtonGroupItem } from "./components/base/button-group/button-group";

// input
export { Input } from "./components/base/input/input";
export { InputGroup, InputPrefix } from "./components/base/input/input-group";
export { Label } from "./components/base/input/label";
export { HintText } from "./components/base/input/hint-text";

// textarea
export { TextArea } from "./components/base/textarea/textarea";

// avatar
export { Avatar } from "./components/base/avatar/avatar";
export { AvatarLabelGroup } from "./components/base/avatar/avatar-label-group";
export { AvatarProfilePhoto } from "./components/base/avatar/avatar-profile-photo";

// badges
export { Badge, BadgeWithDot, BadgeWithIcon, BadgeWithFlag, BadgeWithImage, BadgeWithButton } from "./components/base/badges/badges";
export { BadgeGroup } from "./components/base/badges/badge-groups";

// tooltip
export { Tooltip, TooltipTrigger } from "./components/base/tooltip/tooltip";

// form controls
export { Checkbox } from "./components/base/checkbox/checkbox";
export { Toggle } from "./components/base/toggle/toggle";
export { Slider } from "./components/base/slider/slider";
export { RadioButton, RadioGroup } from "./components/base/radio-buttons/radio-buttons";

// select
export { Select } from "./components/base/select/select";
export { MultiSelect } from "./components/base/select/multi-select";
export { ComboBox } from "./components/base/select/combobox";
export { TagSelect } from "./components/base/select/tag-select";
export { NativeSelect } from "./components/base/select/select-native";
export { SelectItem } from "./components/base/select/select-item";

// dropdown
export { Dropdown } from "./components/base/dropdown/dropdown";

// tags
export { Tag, TagGroup, TagList, TagAvatar } from "./components/base/tags/tags";
export type { TagItem } from "./components/base/tags/tags";

// form
export { Form } from "./components/base/form/form";

// progress
export { ProgressBar } from "./components/base/progress-indicators/progress-indicators";
export { ProgressBarCircle, ProgressBarHalfCircle } from "./components/base/progress-indicators/progress-circles";

// file upload
export { FileTrigger } from "./components/base/file-upload-trigger/file-upload-trigger";

// foundations
export { FeaturedIcon } from "./components/foundations/featured-icon/featured-icon";
export { Dot } from "./components/foundations/dot-icon";

// application
export { LoadingIndicator } from "./components/application/loading-indicator/loading-indicator";

// hooks
export { useClipboard } from "./hooks/use-clipboard";
export { useBreakpoint } from "./hooks/use-breakpoint";
export { useResizeObserver } from "./hooks/use-resize-observer";

// utils
export { cx, sortCx } from "./utils/cx";
export { isReactComponent, isFunctionComponent, isClassComponent, isForwardRefComponent } from "./utils/is-react-component";

declare module "*.webp" {
  const url: string;
  export default url;
}

// Desktop WebViews expose the work-area origin in addition to its dimensions.
interface Screen {
  readonly availLeft: number;
  readonly availTop: number;
}

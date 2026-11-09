import { dispatchWorkflow } from "./publish_github_release.mjs";
dispatchWorkflow("build", process.argv.slice(2));

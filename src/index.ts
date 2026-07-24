#!/usr/bin/env node

import { FlowCli } from "./cli/FlowCli.js";

await new FlowCli().run();

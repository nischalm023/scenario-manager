// Side-effect module: import first in main.ts so instrumentation patches modules before they load.
import { initTelemetry } from './index.js';

await initTelemetry();

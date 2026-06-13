import { round_trip } from './pkg/wasm_sqlite_spike.js';
const result = round_trip();
console.log(JSON.stringify(result, null, 2));

import {installRuntime} from './bridge-runtime.mjs';
import manifest from '@wallet/release-manifest';
installRuntime(manifest);

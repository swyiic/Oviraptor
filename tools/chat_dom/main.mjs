import { createApp } from 'vue';
import Fixture from './Fixture.vue';
import { metrics } from './transport.mjs';
import '../../src/styles.css';

const report = (error) => metrics.errors.push(String(error?.message || error));
window.addEventListener('error', (event) => report(event.error || event.message));
window.addEventListener('unhandledrejection', (event) => report(event.reason));
const app = createApp(Fixture);
app.config.errorHandler = report;
app.mount('#app');

import {createApp} from 'vue';
import {createPageClient} from '../../../packages/app-sdk/src/page-client.mjs';
import Root from './Root.vue';
import './style.css';
const connection={appId:'org.oviraptor.assets',timeoutMs:30000};
const client=createPageClient(connection);
createApp(Root,{client}).mount('#app');
window.addEventListener('pagehide',()=>client.destroy(),{once:true});

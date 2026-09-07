import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "@/App.vue";
import "@/assets/css/main.css";

createApp(App).use(createPinia()).mount("#app");

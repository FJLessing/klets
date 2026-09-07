<script lang="ts" setup>
import { computed } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import LauncherView from "@/views/LauncherView.vue";
import SettingsView from "@/views/SettingsView.vue";

// Both windows load the same bundle; the label decides what to render.
const isSettings = computed(() => getCurrentWindow().label === "settings");
</script>

<template>
	<SettingsView v-if="isSettings" />
	<LauncherView v-else />
</template>

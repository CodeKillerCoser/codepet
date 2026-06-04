import PetApp from "./PetApp.svelte";
import "./styles.css";
import { mount } from "svelte";

mount(PetApp, { target: document.getElementById("pet")! });

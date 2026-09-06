// Review-only semantic mapping, resolved from the repository's installed Radix palette.
import * as radix from '@radix-ui/colors';
import { writeFileSync } from 'node:fs';
const mappings = {
  'bg.canvas':'sand1', 'bg.sidebar':'sand2', 'bg.surface':'sand1',
  'bg.subtle':'sand3', 'bg.overlay':'sand2',
  'text.primary':'sand12', 'text.secondary':'sand11', 'text.muted':'sand11', 'text.disabled':'sand9',
  'border.subtle':'sand6', 'border.strong':'sand9',
  'icon.primary':'sand12', 'icon.secondary':'sand11',
  'interaction.hover':'sand4', 'interaction.pressed':'sand5',
  'selection.bg':'amber3', 'selection.fg':'amber12', 'selection.indicator':'amber11',
  'accent.solid':'amber9', 'accent.hover':'amber10', 'accent.pressed':'amber9',
  'accent.onSolid':'sand12@light', 'accent.text':'amber11', 'focus.ring':'amber11',
  'control.bg':'sand1', 'control.border':'sand9', 'control.disabledBg':'sand3',
  'scrim':'blackA8', 'shadow':'blackA4',
};
for (const [state,palette] of Object.entries({success:'grass',warning:'amber',danger:'tomato',info:'blue',neutral:'sand'})) {
  mappings[`status.${state}.bg`] = `${palette}3`;
  mappings[`status.${state}.fg`] = `${palette}12`;
}
function resolve(ref, theme) {
  const [token,fixed] = ref.split('@');
  const [,palette,alpha] = token.match(/^([a-z]+)(A?)\d+$/);
  const dark = !fixed && theme === 'dark' && palette !== 'black';
  return radix[palette + (dark ? 'Dark' : '') + alpha][token];
}
const themes = Object.fromEntries(['light','dark'].map(theme => [theme,
  Object.fromEntries(Object.entries(mappings).map(([key,ref]) => [key,resolve(ref,theme)]))]));
writeFileSync(new URL('color-tokens.json',import.meta.url),JSON.stringify({
  status:'design proposal; not connected to production',source:'@radix-ui/colors (installed dependency)',mappings,themes,
},null,2)+'\n');
function lum(hex) {
  const channels = [1,3,5].map(i => parseInt(hex.slice(i,i+2),16)/255)
    .map(v => v<=.04045 ? v/12.92 : ((v+.055)/1.055)**2.4);
  return channels.reduce((s,v,i) => s+v*[.2126,.7152,.0722][i],0);
}
const pairs = ['text.primary','text.secondary','text.muted'].flatMap(fg =>
  ['bg.canvas','bg.sidebar','bg.surface','bg.subtle','bg.overlay'].map(bg => [fg,bg,4.5]));
pairs.push(['selection.fg','selection.bg',4.5], ['accent.text','bg.canvas',4.5]);
for (const bg of ['accent.solid','accent.hover','accent.pressed']) pairs.push(['accent.onSolid',bg,4.5]);
for (const s of ['success','warning','danger','info','neutral']) pairs.push([`status.${s}.fg`,`status.${s}.bg`,4.5]);
for (const fg of ['control.border','focus.ring','selection.indicator']) {
  for (const bg of ['bg.canvas','bg.sidebar','control.bg']) pairs.push([fg,bg,3]);
}
const results=[];
for (const [theme,colors] of Object.entries(themes)) {
  for (const [fg,bg,minimum] of pairs) {
    const [low,high]=[lum(colors[fg]),lum(colors[bg])].sort((a,b)=>a-b);
    const ratio=(high+.05)/(low+.05);
    results.push({theme,foreground:fg,background:bg,ratio:Number(ratio.toFixed(2)),minimum,pass:ratio>=minimum});
  }
}
writeFileSync(new URL('contrast-check.json',import.meta.url),JSON.stringify(results,null,2)+'\n');
const failures=results.filter(r=>!r.pass);
console.log(`${results.length-failures.length}/${results.length} intended pairs pass.`);
if(failures.length) { console.log(failures); process.exitCode=1; }

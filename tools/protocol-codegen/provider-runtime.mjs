// Mechanisms are handwritten; every protocol value emitted here is owned by the canonical IDL.
const snake = value => value.replace(/[A-Z]/g,m=>'_'+m.toLowerCase());
const upper = value => snake(value).toUpperCase();
const banner='// Generated from protocol/v2/provider; do not edit.\n';
export function providerRuntimeArtifacts(model,published) {
  const w=model.wire,lines=[banner,'use crate::generated::ProviderTransportLimits;','use crate::TransportError;'];
  const string=(k,v)=>lines.push(`pub const ${k}: &str = ${JSON.stringify(v)};`);
  const number=(k,v,type='usize')=>lines.push(`pub const ${k}: ${type} = ${v};`);
  for(const p of model.profiles)string(p.multiplexing==='yamux-wire-0'?'MUX_PROFILE':p.compression==='zstd'?'ZSTD_PROFILE':'JSONRPC_PROFILE',p.id);
  string('DEFAULT_PROFILE',model.files.get(model.index.profiles).defaultProfile);
  string('TRANSPORT_ENV',model.files.get(model.index.profiles).environmentKey);
  for(const [name,section]of [['BOOTSTRAP',w.bootstrap],['FRAME',w.frame],['MESSAGE',w.message]]){
    for(const [k,v]of Object.entries(section)){
      if(typeof v==='number')number(`${name}_${upper(k)}`,v,['version','confirmation','grant','eventAck'].includes(k)?'u8':'usize');
      else if(k==='magic')lines.push(`pub const ${name}_MAGIC: [u8; 4] = *b${JSON.stringify(v)};`);
    }
  }
  for(const [section,data]of Object.entries({BOOTSTRAP:w.bootstrap,FRAME:w.frame,MESSAGE:w.message})){
    let offset=0;
    for(const field of data.fields){number(`${section}_${upper(field.name)}_OFFSET`,offset);const width={bytes4:4,u8:1,u16:2,u32:4}[field.type];if(!width)throw Error('Unknown wire field type');offset+=width;}
    if(data.headerBytes!==undefined&&offset!==data.headerBytes)throw Error('Wire header size mismatch');
    if(section==='BOOTSTRAP')number('BOOTSTRAP_HEADER_BYTES',offset);
  }
  for(const [k,v]of Object.entries(w.bootstrap.phases))number(`PHASE_${upper(k)}`,v,'u8');
  for(const [k,v]of Object.entries(w.frame.encodings))number(`ENCODING_${upper(k)}`,v,'u8');
  for(const [k,v]of Object.entries(w.message.classes))number(`CLASS_${upper(k)}`,v,'u8');
  for(const [k,v]of Object.entries(w.compression))number(`COMPRESSION_${upper(k)}`,v,k.endsWith('Level')?'i32':'usize');
  lines.push(`pub const FEATURES: &[&str] = &[${w.bootstrap.features.map(v=>JSON.stringify(v)).join(',')}];`);
  for(const [name,schema]of Object.entries(model.limits))number(upper(name),schema.default);
  lines.push('pub fn default_transport_limits() -> ProviderTransportLimits { ProviderTransportLimits {');
  for(const [name,schema]of Object.entries(model.limits))lines.push(`${snake(name)}: ${schema.default},`);
  lines.push('} }','pub fn validate_limits(v: &ProviderTransportLimits) -> Result<(), TransportError> {');
  for(const [name,schema]of Object.entries(model.limits))lines.push(`if v.${snake(name)} < ${schema.minimum} || v.${snake(name)} > ${schema.maximum} { return Err(TransportError::new("invalid negotiated ${name}")); }`);
  function expression(v){
    if(typeof v==='string'){if(!model.limits[v])throw Error('Unknown constraint field '+v);return `v.${snake(v)}`;}
    if(typeof v==='number')return String(v);
    if(v.sum)return '('+v.sum.map(expression).join(' + ')+')';
    if(v.multiply)return '('+v.multiply.map(expression).join(' * ')+')';
    throw Error('Unsupported transport constraint');
  }
  for(const constraint of model.constraints){
    if(!['>=','<='].includes(constraint.operator))throw Error('Unsupported constraint operator');
    lines.push(`if !(${expression(constraint.left)} ${constraint.operator} ${expression(constraint.right)}) { return Err(TransportError::new(${JSON.stringify(constraint.description)})); }`);
  }
  lines.push('Ok(()) }');
  const h=model.lifecycle;
  const heartbeat=banner+`pub const PROVIDER_HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(${h.heartbeatIntervalMs});\npub const PROVIDER_HEARTBEAT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(${h.heartbeatTimeoutMs});\n`;
  // Publish the exact installation Schema for the Host's standard JSON Schema validator.
  const schema=JSON.parse(published.get(model.index.manifestSchema));
  function rustType(field,node){
    if(field==='command')return 'std::path::PathBuf';
    if(field==='pluginId')return 'codepet_provider_sdk::ProviderPluginId';
    if(field==='instanceId')return 'codepet_provider_sdk::ProviderInstanceId';
    if(field==='instances')return 'Vec<PluginInstanceConfig>';
    if(node.type==='string')return 'String';
    if(node.type==='boolean')return 'bool';
    if(node.type==='array')return `Vec<${rustType('',node.items)}>`;
    if(node.type==='object'&&node.additionalProperties?.type==='string')return 'std::collections::BTreeMap<String,String>';
    if(node.type==='object')return 'codepet_provider_sdk::JsonObject';
    throw Error('Unsupported installation DTO '+field);
  }
  function structure(name,properties,required){
    let code=`#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]\n#[serde(rename_all="camelCase",deny_unknown_fields)]\npub struct ${name} {\n`;
    for(const [field,node]of Object.entries(properties)){
      const optional=!required.includes(field)&&node.default===undefined;
      if(field==='command')code+='#[serde(rename="command")]\n';
      if(optional)code+='#[serde(default,skip_serializing_if="Option::is_none")]\n';
      else if(node.default===true)code+='#[serde(default="super::default_true")]\n';
      else if(node.default!==undefined)code+='#[serde(default)]\n';
      const type=rustType(field,node);code+=`pub ${field==='command'?'executable':snake(field)}: ${optional?`Option<${type}>`:type},\n`;
    }
    return code+'}\n';
  }
  const installed=Object.fromEntries(Object.entries(schema.properties).filter(([k])=>!['$schema','manifestVersion','transport'].includes(k)));
  const types=structure('PluginInstanceConfig',schema.properties.instances.items.properties,schema.properties.instances.items.required)+structure('PluginDescriptor',installed,schema.required);
  const raw='#[derive(Clone, Debug, serde::Deserialize)]\n#[serde(rename_all="camelCase",deny_unknown_fields)]\npub(super) struct PluginManifest { pub manifest_version:u32, #[serde(rename="$schema",default)] pub _schema:Option<String>, pub transport:String, #[serde(flatten)] pub descriptor:PluginDescriptor }\n';
  const manifest=banner+types+raw+`pub const PLUGIN_MANIFEST_FILE_NAME: &str = ${JSON.stringify(model.index.pluginManifestFileName)};\npub const PLUGIN_MANIFEST_VERSION: u32 = ${schema.properties.manifestVersion.const};\n`;
  return new Map([
    ['sdk/rust/codepet-transport/src/protocol.rs',lines.join('\n')+'\n'],
    ['sdk/rust/codepet-provider-sdk/src/runtime/protocol.rs',heartbeat],
    ['crates/codepet-host/src/providers/manifest_generated.rs',manifest],
    ['crates/codepet-host/src/providers/manifest.schema.generated.json',JSON.stringify(schema,null,2)+'\n'],
    ['crates/codepet-host/src/notice.schema.generated.json',published.get(model.index.commonSchema)],
  ]);
}

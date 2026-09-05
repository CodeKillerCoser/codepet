const dartReservedWords = new Set([
  "abstract", "as", "assert", "async", "await", "base", "break", "case", "catch",
  "class", "const", "continue", "covariant", "default", "deferred", "do", "dynamic",
  "else", "enum", "export", "extends", "extension", "external", "factory", "false",
  "final", "finally", "for", "Function", "get", "hide", "if", "implements", "import",
  "in", "interface", "is", "late", "library", "mixin", "new", "null", "of", "on",
  "operator", "part", "required", "rethrow", "return", "sealed", "set", "show", "static",
  "super", "switch", "sync", "this", "throw", "true", "try", "typedef", "var", "void",
  "when", "while", "with", "yield",
]);

function fail(message) {
  throw new Error(message);
}

function assert(condition, message) {
  if (!condition) fail(message);
}

function words(value) {
  return value.replace(/([a-z0-9])([A-Z])/g, "$1 $2").split(/[^A-Za-z0-9]+/).filter(Boolean);
}

function pascalCase(value) {
  return words(value).map((word) => word[0].toUpperCase() + word.slice(1)).join("");
}

function camelCase(value) {
  const name = pascalCase(value);
  const result = name[0].toLowerCase() + name.slice(1);
  return dartReservedWords.has(result) ? `${result}Value` : result;
}

function dartString(value) {
  return `'${value.replaceAll("\\", "\\\\").replaceAll("'", "\\'").replaceAll("$", "\\$")}'`;
}

class DartWriter {
  constructor() {
    this.lines = [];
    this.indent = 0;
  }

  line(value = "") {
    this.lines.push(`${"  ".repeat(this.indent)}${value}`);
  }

  block(header, body, trailer = "}") {
    this.line(`${header} {`);
    this.indent += 1;
    body();
    this.indent -= 1;
    this.line(trailer);
  }

  blank() {
    if (this.lines.at(-1) !== "") this.lines.push("");
  }

  toString() {
    return `${this.lines.join("\n").trimEnd()}\n`;
  }
}

function packageIr(ir, packageId) {
  const value = ir.packagesById.get(packageId);
  assert(value, `Dart IR is missing package ${packageId}`);
  return value;
}

function definitionIr(ir, type) {
  const definition = packageIr(ir, type.packageId).definitions.find((value) => value.name === type.name);
  assert(definition, `Dart IR cannot resolve ${type.packageId}.${type.name}`);
  return definition;
}

function nullableType(value) {
  return value.endsWith("?") ? value : `${value}?`;
}

function dartType(type) {
  if (type.kind === "named") return type.name;
  if (type.kind === "nullable") return nullableType(dartType(type.value));
  if (type.kind === "string") return "String";
  if (type.kind === "integer") return "int";
  if (type.kind === "boolean") return "bool";
  if (type.kind === "null") return "Object?";
  if (type.kind === "jsonObject") return "Map<String, Object?>";
  if (type.kind === "map") return `Map<String, ${dartType(type.values)}>`;
  if (type.kind === "list") return `List<${dartType(type.items)}>`;
  fail(`unsupported Dart IR type ${type.kind}`);
}

function constraintArguments(constraints = {}) {
  const values = [];
  if (constraints.minimum !== undefined) values.push(`minimum: ${constraints.minimum}`);
  if (constraints.maximum !== undefined) values.push(`maximum: ${constraints.maximum}`);
  if (constraints.minLength !== undefined) values.push(`minLength: ${constraints.minLength}`);
  if (constraints.maxLength !== undefined) values.push(`maxLength: ${constraints.maxLength}`);
  if (constraints.pattern !== undefined) values.push(`pattern: ${dartString(constraints.pattern)}`);
  if (constraints.minItems !== undefined) values.push(`minItems: ${constraints.minItems}`);
  if (constraints.uniqueItems === true) values.push("uniqueItems: true");
  return values.length > 0 ? `, ${values.join(", ")}` : "";
}

function decodeExpression(type, value, path, ir) {
  if (type.kind === "named") {
    const definition = definitionIr(ir, type);
    if (definition.kind === "alias") return decodeExpression(definition.type, value, path, ir);
    return `${type.name}.fromJson(${value}, path: ${path})`;
  }
  if (type.kind === "nullable") {
    return `${value} == null ? null : ${decodeExpression(type.value, value, path, ir)}`;
  }
  if (type.kind === "string") return `_string(${value}, ${path}${constraintArguments(type.constraints)})`;
  if (type.kind === "integer") return `_integer(${value}, ${path}${constraintArguments(type.constraints)})`;
  if (type.kind === "boolean") return `_boolean(${value}, ${path})`;
  if (type.kind === "null") return `_nullValue(${value}, ${path})`;
  if (type.kind === "jsonObject") return `_jsonObject(${value}, ${path})`;
  if (type.kind === "map") {
    const decoded = decodeExpression(type.values, "item", "itemPath", ir);
    return `_decodeMap<${dartType(type.values)}>(${value}, ${path}, (item, itemPath) => ${decoded})`;
  }
  if (type.kind === "list") {
    const itemPath = "itemPath";
    const decode = decodeExpression(type.items, "item", itemPath, ir);
    const encode = encodeExpression(type.items, "item", `${path} + '[]'`, ir);
    return `_decodeList<${dartType(type.items)}>(${value}, ${path}, (item, itemPath) => ${decode}${constraintArguments(type.constraints)}, encodeItem: (item) => ${encode})`;
  }
  fail(`cannot decode Dart IR type ${type.kind}`);
}

function validateExpression(type, value, path, ir) {
  if (type.kind === "named") {
    const definition = definitionIr(ir, type);
    return definition.kind === "alias" ? validateExpression(definition.type, value, path, ir) : value;
  }
  if (type.kind === "nullable") {
    return `${value} == null ? null : ${validateExpression(type.value, value, path, ir)}`;
  }
  if (type.kind === "string") return `_string(${value}, ${path}${constraintArguments(type.constraints)})`;
  if (type.kind === "integer") return `_integer(${value}, ${path}${constraintArguments(type.constraints)})`;
  if (type.kind === "boolean") return value;
  if (type.kind === "null") return `_nullValue(${value}, ${path})`;
  if (type.kind === "jsonObject") return `_jsonObject(${value}, ${path})`;
  if (type.kind === "map") {
    const validated = validateExpression(type.values, "item", "itemPath", ir);
    return `_freezeMap<${dartType(type.values)}>(${value}, ${path}, (item, itemPath) => ${validated})`;
  }
  if (type.kind === "list") {
    const validate = validateExpression(type.items, "item", "itemPath", ir);
    const encode = encodeExpression(type.items, "item", `${path} + '[]'`, ir);
    return `_freezeList<${dartType(type.items)}>(${value}, ${path}, (item, itemPath) => ${validate}${constraintArguments(type.constraints)}, encodeItem: (item) => ${encode})`;
  }
  fail(`cannot validate Dart IR type ${type.kind}`);
}

function encodeExpression(type, value, path, ir) {
  if (type.kind === "named") {
    const definition = definitionIr(ir, type);
    if (definition.kind === "alias") return encodeExpression(definition.type, value, path, ir);
    return `${value}.toJson()`;
  }
  if (type.kind === "nullable") {
    return `${value} == null ? null : ${encodeExpression(type.value, `${value}!`, path, ir)}`;
  }
  if (["string", "integer", "boolean", "null"].includes(type.kind)) return value;
  if (type.kind === "jsonObject") return `_encodeJsonObject(${value}, ${path})`;
  if (type.kind === "map") {
    const item = encodeExpression(type.values, "item", `${path} + '[]'`, ir);
    return `${value}.map((key, item) => MapEntry(key, ${item}))`;
  }
  if (type.kind === "list") {
    const item = encodeExpression(type.items, "item", `${path} + '[]'`, ir);
    return `${value}.map((item) => ${item}).toList(growable: false)`;
  }
  fail(`cannot encode Dart IR type ${type.kind}`);
}

function fieldType(field) {
  const base = dartType(field.type);
  return field.required ? base : nullableType(base);
}

function unionParents(pkg) {
  const parents = new Map();
  for (const definition of pkg.definitions.filter((value) => value.kind === "union")) {
    for (const variant of definition.variants) {
      assert(variant.packageId === pkg.id, `${pkg.id}.${definition.name} cannot extend an external Dart union variant`);
      assert(!parents.has(variant.name), `${pkg.id}.${variant.name} belongs to more than one Dart union`);
      parents.set(variant.name, definition.name);
    }
  }
  return parents;
}

function emitEnum(writer, definition) {
  writer.block(`enum ${definition.name}`, () => {
    definition.values.forEach((value, index) => {
      const suffix = index === definition.values.length - 1 ? ";" : ",";
      writer.line(`${camelCase(value)}(${dartString(value)})${suffix}`);
    });
    writer.blank();
    writer.line(`const ${definition.name}(this.wireValue);`);
    writer.blank();
    writer.line("final String wireValue;");
    writer.blank();
    writer.block(`static ${definition.name} fromJson(Object? value, {String path = ${dartString(definition.name)}})`, () => {
      writer.line(`final wireValue = _string(value, path${constraintArguments(definition.constraints)});`);
      writer.block("for (final candidate in values)", () => {
        writer.line("if (candidate.wireValue == wireValue) return candidate;");
      });
      writer.line(`throw ProtocolCodecException(path, 'expected one of: ${definition.values.join(", ")}');`);
    });
    writer.blank();
    writer.line("String toJson() => wireValue;");
  });
}

function emitUnion(writer, definition) {
  assert(
    definition.discriminator,
    `${definition.packageId}.${definition.name} is an untagged or ambiguous oneOf; Dart requires one common required singleton-enum discriminator`,
  );
  writer.block(`sealed class ${definition.name}`, () => {
    writer.line(`const ${definition.name}();`);
    writer.blank();
    writer.block(`factory ${definition.name}.fromJson(Object? value, {String path = ${dartString(definition.name)}})`, () => {
      writer.line("final json = _object(value, path);");
      writer.line(`final discriminator = _required(json, ${dartString(definition.discriminator.field)}, path);`);
      writer.block("switch (discriminator)", () => {
        for (const variant of definition.discriminator.variants) {
          writer.line(`case ${dartString(variant.value)}:`);
          writer.indent += 1;
          writer.line(`return ${variant.name}.fromJson(json, path: path);`);
          writer.indent -= 1;
        }
        writer.line("default:");
        writer.indent += 1;
        writer.line(`throw ProtocolCodecException('\$path.${definition.discriminator.field}', 'unknown ${definition.name} discriminator: \$discriminator');`);
        writer.indent -= 1;
      });
    });
    writer.blank();
    writer.line("Map<String, Object?> toJson();");
  });
}

function emitObject(writer, definition, parent, ir) {
  const classHeader = `final class ${definition.name}${parent ? ` extends ${parent}` : ""}`;
  writer.block(classHeader, () => {
    const parameters = definition.fields.map((field) => {
      const required = field.required ? "required " : "";
      return `${required}${fieldType(field)} ${camelCase(field.wireName)}`;
    });
    writer.line(parameters.length > 0 ? `factory ${definition.name}({` : `factory ${definition.name}()`);
    if (parameters.length > 0) {
      writer.indent += 1;
      for (const parameter of parameters) writer.line(`${parameter},`);
      writer.indent -= 1;
    }
    writer.block(parameters.length > 0 ? "})" : "", () => {
      for (const field of definition.fields) {
        const name = camelCase(field.wireName);
        const path = dartString(`${definition.name}.${field.wireName}`);
        const expression = field.required
          ? validateExpression(field.type, name, path, ir)
          : `${name} == null ? null : ${validateExpression(field.type, name, path, ir)}`;
        writer.line(`final validated${pascalCase(field.wireName)} = ${expression};`);
      }
      writer.line(`return ${definition.name}._(`);
      writer.indent += 1;
      for (const field of definition.fields) {
        writer.line(`${camelCase(field.wireName)}: validated${pascalCase(field.wireName)},`);
      }
      writer.indent -= 1;
      writer.line(");");
    });
    writer.blank();
    if (definition.fields.length > 0) {
      writer.line(`${definition.name}._({`);
      writer.indent += 1;
      for (const field of definition.fields) {
        writer.line(`required this.${camelCase(field.wireName)},`);
      }
      writer.indent -= 1;
      writer.line("});");
    } else {
      writer.line(`${definition.name}._();`);
    }
    writer.blank();
    for (const field of definition.fields) writer.line(`final ${fieldType(field)} ${camelCase(field.wireName)};`);
    writer.blank();
    writer.block(`factory ${definition.name}.fromJson(Object? value, {String path = ${dartString(definition.name)}})`, () => {
      writer.line("final json = _object(value, path);");
      if (definition.closed) {
        writer.line(`_expectKeys(json, const {${definition.fields.map((field) => dartString(field.wireName)).join(", ")}}, path);`);
      }
      if (definition.fields.length === 0) {
        writer.line(`return ${definition.name}();`);
      } else {
        writer.line(`return ${definition.name}(`);
        writer.indent += 1;
        for (const field of definition.fields) {
          const name = camelCase(field.wireName);
          const fieldPath = `'\$path.${field.wireName}'`;
          const value = field.required
            ? `_required(json, ${dartString(field.wireName)}, path)`
            : `json[${dartString(field.wireName)}]`;
          const decoded = decodeExpression(field.type, value, fieldPath, ir);
          const expression = field.required
            ? decoded
            : `json.containsKey(${dartString(field.wireName)}) && json[${dartString(field.wireName)}] != null ? ${decoded} : null`;
          writer.line(`${name}: ${expression},`);
        }
        writer.indent -= 1;
        writer.line(");");
      }
    });
    writer.blank();
    if (parent) writer.line("@override");
    writer.block("Map<String, Object?> toJson() =>", () => {
      for (const field of definition.fields) {
        const name = camelCase(field.wireName);
        const encoded = encodeExpression(field.type, name, dartString(`${definition.name}.${field.wireName}`), ir);
        if (field.required) writer.line(`${dartString(field.wireName)}: ${encoded},`);
        else writer.line(`if (${name} != null) ${dartString(field.wireName)}: ${encodeExpression(field.type, `${name}!`, dartString(`${definition.name}.${field.wireName}`), ir)},`);
      }
    }, "};");
    writer.blank();
    writer.line("@override");
    const display = definition.fields.map((field) => {
      const name = camelCase(field.wireName);
      return field.sensitive ? `${field.wireName}: <redacted>` : `${field.wireName}: \$${name}`;
    }).join(", ");
    writer.line(`String toString() => ${dartString(`${definition.name}(${display})`).replaceAll("\\$", "$")};`);
  });
}

function emitAlias(writer, definition, ir) {
  writer.line(`typedef ${definition.name} = ${dartType(definition.type)};`);
  writer.blank();
  writer.line(`${definition.name} decode${definition.name}(Object? value, {String path = ${dartString(definition.name)}}) => ${decodeExpression(definition.type, "value", "path", ir)};`);
  writer.block(`Object? encode${definition.name}(${definition.name} value, {String path = ${dartString(definition.name)}})`, () => {
    writer.line(`final checked = ${validateExpression(definition.type, "value", "path", ir)};`);
    writer.line(`return ${encodeExpression(definition.type, "checked", "path", ir)};`);
  });
}

function emitCodecHelpers(writer, includeException) {
  if (includeException) {
    writer.block("final class ProtocolCodecException implements Exception", () => {
      writer.line("const ProtocolCodecException(this.path, this.message);");
      writer.blank();
      writer.line("final String path;");
      writer.line("final String message;");
      writer.blank();
      writer.line("@override");
      writer.line("String toString() => 'ProtocolCodecException at $path: $message';");
    });
    writer.blank();
  }
  writer.block("Map<String, Object?> _object(Object? value, String path)", () => {
    writer.line("if (value is! Map) throw ProtocolCodecException(path, 'expected an object');");
    writer.line("final result = <String, Object?>{};");
    writer.block("for (final entry in value.entries)", () => {
      writer.line("if (entry.key is! String) throw ProtocolCodecException(path, 'object keys must be strings');");
      writer.line("result[entry.key as String] = entry.value;");
    });
    writer.line("return result;");
  });
  writer.blank();
  writer.block("void _expectKeys(Map<String, Object?> value, Set<String> allowed, String path)", () => {
    writer.block("for (final key in value.keys)", () => {
      writer.line("if (!allowed.contains(key)) throw ProtocolCodecException('$path.$key', 'unknown field');");
    });
  });
  writer.blank();
  writer.block("Object? _required(Map<String, Object?> value, String key, String path)", () => {
    writer.line("if (!value.containsKey(key)) throw ProtocolCodecException('$path.$key', 'required field is missing');");
    writer.line("return value[key];");
  });
  writer.blank();
  writer.line("Object? _nullValue(Object? value, String path) {");
  writer.line("  if (value != null) throw ProtocolCodecException(path, 'expected null');");
  writer.line("  return null;");
  writer.line("}");
  writer.blank();
  writer.block("String _string(Object? value, String path, {int? minLength, int? maxLength, String? pattern})", () => {
    writer.line("if (value is! String) throw ProtocolCodecException(path, 'expected a string');");
    writer.line("if (minLength != null && value.length < minLength) throw ProtocolCodecException(path, 'string is shorter than $minLength characters');");
    writer.line("if (maxLength != null && value.length > maxLength) throw ProtocolCodecException(path, 'string is longer than $maxLength characters');");
    writer.line("if (pattern != null && !RegExp(pattern).hasMatch(value)) throw ProtocolCodecException(path, 'string does not match $pattern');");
    writer.line("return value;");
  });
  writer.blank();
  writer.block("int _integer(Object? value, String path, {int? minimum, int? maximum})", () => {
    writer.line("if (value is! int) throw ProtocolCodecException(path, 'expected an integer');");
    writer.line("if (minimum != null && value < minimum) throw ProtocolCodecException(path, 'integer is below $minimum');");
    writer.line("if (maximum != null && value > maximum) throw ProtocolCodecException(path, 'integer is above $maximum');");
    writer.line("return value;");
  });
  writer.blank();
  writer.block("bool _boolean(Object? value, String path)", () => {
    writer.line("if (value is! bool) throw ProtocolCodecException(path, 'expected a boolean');");
    writer.line("return value;");
  });
  writer.blank();
  writer.block("Object? _jsonValue(Object? value, String path)", () => {
    writer.line("if (value == null || value is bool || value is num || value is String) return value;");
    writer.block("if (value is List)", () => {
      writer.line("return List<Object?>.unmodifiable(value.indexed.map((entry) => _jsonValue(entry.$2, '$path[${entry.$1}]')));");
    });
    writer.block("if (value is Map)", () => {
      writer.line("return Map<String, Object?>.unmodifiable(_object(value, path).map((key, item) => MapEntry(key, _jsonValue(item, '$path.$key'))));");
    });
    writer.line("throw ProtocolCodecException(path, 'expected a JSON value');");
  });
  writer.blank();
  writer.line("Map<String, Object?> _jsonObject(Object? value, String path) => _jsonValue(_object(value, path), path) as Map<String, Object?>;");
  writer.line("Map<String, Object?> _encodeJsonObject(Map<String, Object?> value, String path) => _jsonObject(value, path);");
  writer.blank();
  writer.block("Map<String, T> _freezeMap<T>(Map<String, T> values, String path, T Function(T, String) validate)", () => {
    writer.line("return Map<String, T>.unmodifiable(values.map((key, item) => MapEntry(key, validate(item, '$path.$key'))));");
  });
  writer.blank();
  writer.block("Map<String, T> _decodeMap<T>(Object? value, String path, T Function(Object?, String) decode)", () => {
    writer.line("final object = _object(value, path);");
    writer.line("return Map<String, T>.unmodifiable(object.map((key, item) => MapEntry(key, decode(item, '$path.$key'))));");
  });
  writer.blank();
  writer.block("List<T> _freezeList<T>(Iterable<T> values, String path, T Function(T, String) validate, {int? minItems, bool uniqueItems = false, required Object? Function(T) encodeItem})", () => {
    writer.line("final result = List<T>.unmodifiable(values.indexed.map((entry) => validate(entry.$2, '$path[${entry.$1}]')));");
    writer.line("if (minItems != null && result.length < minItems) throw ProtocolCodecException(path, 'array has fewer than $minItems items');");
    writer.block("if (uniqueItems)", () => {
      writer.line("final encoded = result.map((item) => jsonEncode(encodeItem(item))).toSet();");
      writer.line("if (encoded.length != result.length) throw ProtocolCodecException(path, 'array contains duplicate items');");
    });
    writer.line("return result;");
  });
  writer.blank();
  writer.block("List<T> _decodeList<T>(Object? value, String path, T Function(Object?, String) decode, {int? minItems, bool uniqueItems = false, required Object? Function(T) encodeItem})", () => {
    writer.line("if (value is! List) throw ProtocolCodecException(path, 'expected an array');");
    writer.line("return _freezeList<T>(value.indexed.map((entry) => decode(entry.$2, '$path[${entry.$1}]')), path, (item, _) => item, minItems: minItems, uniqueItems: uniqueItems, encodeItem: encodeItem);");
  });
}

function emitProtocolMetadata(writer, pkg, ir) {
  const service = pkg.service;
  writer.line(`const int protocolVersion = ${service.version};`);
  if (service.kind === "json-rpc-2.0") writer.line(`const String jsonRpcVersion = ${dartString(service.jsonRpcVersion)};`);
  writer.blank();
  writer.block("enum ProtocolIdempotency", () => {
    writer.line("safe,");
    writer.line("idempotent,");
    writer.line("nonIdempotent,");
  });
  writer.blank();
  writer.block("enum ProtocolMethod", () => {
    service.methods.forEach((method, index) => {
      const capability = method.capability
        ? `${service.capabilityType.name}.${camelCase(method.capability)}`
        : "null";
      const suffix = index === service.methods.length - 1 ? ";" : ",";
      writer.line(`${camelCase(method.name)}(${dartString(method.name)}, direction: ${dartString(method.direction)}, idempotency: ProtocolIdempotency.${method.idempotency}, capability: ${capability}, requestType: ${method.requestType.name}, responseType: ${method.responseType.name})${suffix}`);
    });
    writer.blank();
    writer.line("const ProtocolMethod(this.wireName, {required this.direction, required this.idempotency, required this.capability, required this.requestType, required this.responseType});");
    writer.blank();
    writer.line("final String wireName;");
    writer.line("final String direction;");
    writer.line("final ProtocolIdempotency idempotency;");
    writer.line(`final ${service.capabilityType.name}? capability;`);
    writer.line("final Type requestType;");
    writer.line("final Type responseType;");
    writer.blank();
    writer.block("static ProtocolMethod fromJson(Object? value, {String path = 'ProtocolMethod'})", () => {
      writer.line("final wireValue = _string(value, path);");
      writer.block("for (final candidate in values)", () => writer.line("if (candidate.wireName == wireValue) return candidate;"));
      writer.line("throw ProtocolCodecException(path, 'unknown protocol method: $wireValue');");
    });
    writer.blank();
    writer.line("String toJson() => wireName;");
  });
  writer.blank();
  writer.block("enum ProtocolEventName", () => {
    service.events.forEach((event, index) => {
      const suffix = index === service.events.length - 1 ? ";" : ",";
      writer.line(`${camelCase(event.name)}(${dartString(event.name)}, direction: ${dartString(event.direction)}, delivery: ${dartString(event.delivery)}, scope: ${dartString(event.scope)}, payloadType: ${event.payloadType.name})${suffix}`);
    });
    writer.blank();
    writer.line("const ProtocolEventName(this.wireName, {required this.direction, required this.delivery, required this.scope, required this.payloadType});");
    writer.blank();
    writer.line("final String wireName;");
    writer.line("final String direction;");
    writer.line("final String delivery;");
    writer.line("final String scope;");
    writer.line("final Type payloadType;");
    writer.blank();
    writer.block("static ProtocolEventName fromJson(Object? value, {String path = 'ProtocolEventName'})", () => {
      writer.line("final wireValue = _string(value, path);");
      writer.block("for (final candidate in values)", () => writer.line("if (candidate.wireName == wireValue) return candidate;"));
      writer.line("throw ProtocolCodecException(path, 'unknown protocol event: $wireValue');");
    });
    writer.blank();
    writer.line("String toJson() => wireName;");
  });
  writer.blank();
  if (service.kind === "codepet-envelope") emitProtocolCodec(writer, pkg, ir);
  else if (service.kind === "json-rpc-2.0") emitJsonRpcProtocolCodec(writer, pkg, ir);
  else fail(`${pkg.id} Dart generator does not support service transport ${service.kind}`);
}

function emitTypedSwitch(writer, entries, selector, typeKey, value, path, ir, mode) {
  writer.block(`switch (${selector})`, () => {
    for (const entry of entries) {
      const enumType = typeKey === "payloadType" ? "ProtocolEventName" : "ProtocolMethod";
      const type = entry[typeKey];
      writer.line(`case ${enumType}.${camelCase(entry.name)}:`);
      writer.indent += 1;
      if (mode === "decode") writer.line(`return ${decodeExpression(type, value, path, ir)};`);
      else {
        writer.line(`if (${value} is! ${type.name}) throw ProtocolCodecException(${path}, 'expected ${type.name}');`);
        writer.line(`return ${encodeExpression(type, value, path, ir)};`);
      }
      writer.indent -= 1;
    }
  });
}

function emitJsonRpcProtocolCodec(writer, pkg, ir) {
  const service = pkg.service;
  const cursorField = service.eventCursorField;
  const traceField = service.traceContextField;
  const traceType = service.traceContextType;
  const traceParameter = traceType ? `, ${traceType.name}? traceContext` : "";
  const traceArgument = traceType ? ", traceContext: traceContext" : "";
  const traceConstructorField = traceType ? ", this.traceContext" : "";
  const traceAllowedKey = traceField ? `, ${dartString(traceField)}` : "";
  const traceDecode = traceType
    ? `, traceContext: json[${dartString(traceField)}] == null ? null : ${decodeExpression(traceType, `json[${dartString(traceField)}]`, `'$path.${traceField}'`, ir)}`
    : "";
  const traceJson = traceType ? `, if (traceContext != null) ${dartString(traceField)}: traceContext!.toJson()` : "";
  writer.block("Object _decodeRequestParams(ProtocolMethod method, Object? value, String path)", () => {
    emitTypedSwitch(writer, service.methods, "method", "requestType", "value", "path", ir, "decode");
  });
  writer.blank();
  writer.block("Map<String, Object?> _encodeRequestParams(ProtocolMethod method, Object value, String path)", () => {
    emitTypedSwitch(writer, service.methods, "method", "requestType", "value", "path", ir, "encode");
  });
  writer.blank();
  writer.block("Object _decodeResponseResult(ProtocolMethod method, Object? value, String path)", () => {
    emitTypedSwitch(writer, service.methods, "method", "responseType", "value", "path", ir, "decode");
  });
  writer.blank();
  writer.block("Map<String, Object?> _encodeResponseResult(ProtocolMethod method, Object value, String path)", () => {
    emitTypedSwitch(writer, service.methods, "method", "responseType", "value", "path", ir, "encode");
  });
  writer.blank();
  writer.block("Object _decodeEventPayload(ProtocolEventName event, Object? value, String path)", () => {
    emitTypedSwitch(writer, service.events, "event", "payloadType", "value", "path", ir, "decode");
  });
  writer.blank();
  writer.block("Map<String, Object?> _encodeEventPayload(ProtocolEventName event, Object value, String path)", () => {
    emitTypedSwitch(writer, service.events, "event", "payloadType", "value", "path", ir, "encode");
  });
  writer.blank();
  writer.block("final class ProtocolRequestEnvelope", () => {
    writer.block(`factory ProtocolRequestEnvelope({required RequestId id, required ProtocolMethod method, required Object params${traceParameter}})`, () => {
      writer.line(`final checkedId = ${validateExpression(service.requestIdType, "id", dartString("id"), ir)};`);
      writer.line("_encodeRequestParams(method, params, 'params');");
      writer.line(`return ProtocolRequestEnvelope._(id: checkedId, method: method, params: params${traceArgument});`);
    });
    writer.blank();
    writer.line(`const ProtocolRequestEnvelope._({required this.id, required this.method, required this.params${traceConstructorField}});`);
    writer.line("final RequestId id;");
    writer.line("final ProtocolMethod method;");
    writer.line("final Object params;");
    if (traceType) writer.line(`final ${traceType.name}? traceContext;`);
    writer.blank();
    writer.block("factory ProtocolRequestEnvelope.fromJson(Object? value, {String path = 'ProtocolRequestEnvelope'})", () => {
      writer.line("final json = _object(value, path);");
      writer.line(`_expectKeys(json, const {'jsonrpc', 'id', 'method', 'params'${traceAllowedKey}}, path);`);
      writer.line("if (_string(_required(json, 'jsonrpc', path), '$path.jsonrpc') != jsonRpcVersion) throw ProtocolCodecException('$path.jsonrpc', 'expected JSON-RPC 2.0');");
      writer.line("final method = ProtocolMethod.fromJson(_required(json, 'method', path), path: '$path.method');");
      writer.line(`return ProtocolRequestEnvelope(id: ${decodeExpression(service.requestIdType, "_required(json, 'id', path)", "'$path.id'", ir)}, method: method, params: _decodeRequestParams(method, _required(json, 'params', path), '$path.params')${traceDecode});`);
    });
    writer.blank();
    writer.line(`Map<String, Object?> toJson() => {'jsonrpc': jsonRpcVersion, 'id': id, 'method': method.toJson(), 'params': _encodeRequestParams(method, params, 'params')${traceJson}};`);
  });
  writer.blank();
  writer.block("sealed class ProtocolResponsePayload", () => writer.line("const ProtocolResponsePayload();"));
  writer.blank();
  writer.block("final class ProtocolSuccess extends ProtocolResponsePayload", () => {
    writer.line("const ProtocolSuccess(this.result);");
    writer.line("final Object result;");
  });
  writer.blank();
  writer.block("final class ProtocolFailure extends ProtocolResponsePayload", () => {
    writer.line("const ProtocolFailure(this.error);");
    writer.line("final RpcError error;");
  });
  writer.blank();
  writer.block("final class ProtocolResponseEnvelope", () => {
    writer.block(`factory ProtocolResponseEnvelope({required RequestId id, required ProtocolMethod method, required ProtocolResponsePayload response${traceParameter}})`, () => {
      writer.line(`final checkedId = ${validateExpression(service.requestIdType, "id", dartString("id"), ir)};`);
      writer.line("if (response is ProtocolSuccess) _encodeResponseResult(method, response.result, 'result');");
      writer.line(`return ProtocolResponseEnvelope._(id: checkedId, method: method, response: response${traceArgument});`);
    });
    writer.blank();
    writer.line(`const ProtocolResponseEnvelope._({required this.id, required this.method, required this.response${traceConstructorField}});`);
    writer.line("final RequestId id;");
    writer.line("final ProtocolMethod method;");
    writer.line("final ProtocolResponsePayload response;");
    if (traceType) writer.line(`final ${traceType.name}? traceContext;`);
    writer.blank();
    writer.block("factory ProtocolResponseEnvelope.fromJson(Object? value, {required ProtocolMethod method, String path = 'ProtocolResponseEnvelope'})", () => {
      writer.line("final json = _object(value, path);");
      writer.line("if (_string(_required(json, 'jsonrpc', path), '$path.jsonrpc') != jsonRpcVersion) throw ProtocolCodecException('$path.jsonrpc', 'expected JSON-RPC 2.0');");
      writer.line("final hasResult = json.containsKey('result');");
      writer.line("final hasError = json.containsKey('error');");
      writer.line("if (hasResult == hasError) throw ProtocolCodecException(path, 'response must contain exactly one of result or error');");
      writer.line(`_expectKeys(json, hasResult ? const {'jsonrpc', 'id', 'result'${traceAllowedKey}} : const {'jsonrpc', 'id', 'error'${traceAllowedKey}}, path);`);
      writer.line("final response = hasResult ? ProtocolSuccess(_decodeResponseResult(method, _required(json, 'result', path), '$path.result')) : ProtocolFailure(RpcError.fromJson(_required(json, 'error', path), path: '$path.error'));");
      writer.line(`return ProtocolResponseEnvelope(id: ${decodeExpression(service.requestIdType, "_required(json, 'id', path)", "'$path.id'", ir)}, method: method, response: response${traceDecode});`);
    });
    writer.blank();
    writer.block("Map<String, Object?> toJson()", () => {
      writer.line("return switch (response) {");
      writer.line(`  ProtocolSuccess(:final result) => {'jsonrpc': jsonRpcVersion, 'id': id, 'result': _encodeResponseResult(method, result, 'result')${traceJson}},`);
      writer.line(`  ProtocolFailure(:final error) => {'jsonrpc': jsonRpcVersion, 'id': id, 'error': error.toJson()${traceJson}},`);
      writer.line("};");
    });
  });
  writer.blank();
  writer.block("final class ProtocolEventEnvelope", () => {
    writer.block(`factory ProtocolEventEnvelope({required EventCursor eventCursor, required ProtocolEventName event, required Object payload${traceParameter}})`, () => {
      writer.line(`final checkedCursor = ${validateExpression(service.eventCursorType, "eventCursor", dartString(cursorField), ir)};`);
      writer.line("_encodeEventPayload(event, payload, 'params.payload');");
      writer.line(`return ProtocolEventEnvelope._(eventCursor: checkedCursor, event: event, payload: payload${traceArgument});`);
    });
    writer.blank();
    writer.line(`const ProtocolEventEnvelope._({required this.eventCursor, required this.event, required this.payload${traceConstructorField}});`);
    writer.line("final EventCursor eventCursor;");
    writer.line("final ProtocolEventName event;");
    writer.line("final Object payload;");
    if (traceType) writer.line(`final ${traceType.name}? traceContext;`);
    writer.blank();
    writer.block("factory ProtocolEventEnvelope.fromJson(Object? value, {String path = 'ProtocolEventEnvelope'})", () => {
      writer.line("final json = _object(value, path);");
      writer.line(`_expectKeys(json, const {'jsonrpc', 'method', 'params'${traceAllowedKey}}, path);`);
      writer.line("if (_string(_required(json, 'jsonrpc', path), '$path.jsonrpc') != jsonRpcVersion) throw ProtocolCodecException('$path.jsonrpc', 'expected JSON-RPC 2.0');");
      writer.line("final event = ProtocolEventName.fromJson(_required(json, 'method', path), path: '$path.method');");
      writer.line("final params = _object(_required(json, 'params', path), '$path.params');");
      writer.line(`_expectKeys(params, const {${dartString(cursorField)}, 'payload'}, '$path.params');`);
      writer.line(`return ProtocolEventEnvelope(eventCursor: ${decodeExpression(service.eventCursorType, `_required(params, ${dartString(cursorField)}, '$path.params')`, `'$path.params.${cursorField}'`, ir)}, event: event, payload: _decodeEventPayload(event, _required(params, 'payload', '$path.params'), '$path.params.payload')${traceDecode});`);
    });
    writer.blank();
    writer.line(`Map<String, Object?> toJson() => {'jsonrpc': jsonRpcVersion, 'method': event.toJson(), 'params': {${dartString(cursorField)}: eventCursor, 'payload': _encodeEventPayload(event, payload, 'params.payload')}${traceJson}};`);
  });
  writer.blank();
  writer.line("ProtocolRequestEnvelope decodeProtocolRequest(String source) => ProtocolRequestEnvelope.fromJson(jsonDecode(source));");
  writer.line("String encodeProtocolRequest(ProtocolRequestEnvelope value) => jsonEncode(value.toJson());");
  writer.line("ProtocolResponseEnvelope decodeProtocolResponse(String source, {required ProtocolMethod method}) => ProtocolResponseEnvelope.fromJson(jsonDecode(source), method: method);");
  writer.line("String encodeProtocolResponse(ProtocolResponseEnvelope value) => jsonEncode(value.toJson());");
  writer.line("ProtocolEventEnvelope decodeProtocolEvent(String source) => ProtocolEventEnvelope.fromJson(jsonDecode(source));");
  writer.line("String encodeProtocolEvent(ProtocolEventEnvelope value) => jsonEncode(value.toJson());");
  writer.blank();
  writer.block("abstract interface class ProtocolTransport", () => writer.line("Future<Object?> request(Map<String, Object?> request);"));
  if (traceType) {
    writer.blank();
    writer.block("abstract interface class ProtocolClientInstrumentation", () => {
      writer.line(`Future<T> traceRequest<T>({required ProtocolMethod method, required RequestId requestId, required Future<T> Function(${traceType.name}? traceContext) invoke});`);
    });
    writer.blank();
    writer.block("final class NoopProtocolClientInstrumentation implements ProtocolClientInstrumentation", () => {
      writer.line("const NoopProtocolClientInstrumentation();");
      writer.line("@override");
      writer.line(`Future<T> traceRequest<T>({required ProtocolMethod method, required RequestId requestId, required Future<T> Function(${traceType.name}? traceContext) invoke}) => invoke(null);`);
    });
  }
  writer.blank();
  writer.block("final class ProtocolRemoteException implements Exception", () => {
    writer.line("const ProtocolRemoteException(this.error);");
    writer.line("final RpcError error;");
    writer.line("@override");
    writer.line("String toString() => 'ProtocolRemoteException(${error.code}): ${error.message}';");
  });
  writer.blank();
  writer.block("final class ProtocolClient", () => {
    writer.line(`const ProtocolClient(this.transport, {required this.requestIdFactory${traceType ? ", this.instrumentation = const NoopProtocolClientInstrumentation()" : ""}});`);
    writer.line("final ProtocolTransport transport;");
    writer.line("final RequestId Function() requestIdFactory;");
    if (traceType) writer.line("final ProtocolClientInstrumentation instrumentation;");
    writer.blank();
    writer.block("Future<TResponse> _request<TResponse>(ProtocolMethod method, Object request) async", () => {
      writer.line("final id = requestIdFactory();");
      if (traceType) writer.line("return instrumentation.traceRequest<TResponse>(method: method, requestId: id, invoke: (traceContext) async {");
      if (traceType) writer.indent += 1;
      writer.line(`final envelope = ProtocolRequestEnvelope(id: id, method: method, params: request${traceArgument});`);
      writer.line("final response = ProtocolResponseEnvelope.fromJson(await transport.request(envelope.toJson()), method: method);");
      writer.line("if (response.id != id) throw ProtocolCodecException('id', 'response id does not match request id');");
      writer.line("return switch (response.response) {");
      writer.line("  ProtocolSuccess(:final result) when result is TResponse => result as TResponse,");
      writer.line("  ProtocolSuccess() => throw ProtocolCodecException('result', 'response result has the wrong generated type'),");
      writer.line("  ProtocolFailure(:final error) => throw ProtocolRemoteException(error),");
      writer.line("};");
      if (traceType) writer.indent -= 1;
      if (traceType) writer.line("});");
    });
    for (const method of service.methods) {
      writer.blank();
      writer.line(`Future<${method.responseType.name}> ${camelCase(method.name)}(${method.requestType.name} request) => _request<${method.responseType.name}>(ProtocolMethod.${camelCase(method.name)}, request);`);
    }
  });
}

function emitProtocolCodec(writer, pkg, ir) {
  const service = pkg.service;
  const versionField = service.versionField;
  const requestDiscriminator = service.requestDiscriminator;
  const responseDiscriminator = service.responseDiscriminator;
  const eventDiscriminator = service.eventDiscriminator;
  const cursorField = service.eventCursorField;
  writer.block("Object _decodeRequestParams(ProtocolMethod method, Object? value, String path)", () => {
    emitTypedSwitch(writer, service.methods, "method", "requestType", "value", "path", ir, "decode");
  });
  writer.blank();
  writer.block("Map<String, Object?> _encodeRequestParams(ProtocolMethod method, Object value, String path)", () => {
    emitTypedSwitch(writer, service.methods, "method", "requestType", "value", "path", ir, "encode");
  });
  writer.blank();
  writer.block("Object _decodeResponseResult(ProtocolMethod method, Object? value, String path)", () => {
    emitTypedSwitch(writer, service.methods, "method", "responseType", "value", "path", ir, "decode");
  });
  writer.blank();
  writer.block("Map<String, Object?> _encodeResponseResult(ProtocolMethod method, Object value, String path)", () => {
    emitTypedSwitch(writer, service.methods, "method", "responseType", "value", "path", ir, "encode");
  });
  writer.blank();
  writer.block("Object _decodeEventPayload(ProtocolEventName event, Object? value, String path)", () => {
    emitTypedSwitch(writer, service.events, "event", "payloadType", "value", "path", ir, "decode");
  });
  writer.blank();
  writer.block("Map<String, Object?> _encodeEventPayload(ProtocolEventName event, Object value, String path)", () => {
    emitTypedSwitch(writer, service.events, "event", "payloadType", "value", "path", ir, "encode");
  });
  writer.blank();
  writer.block("final class ProtocolRequestEnvelope", () => {
    writer.block("factory ProtocolRequestEnvelope({required RequestId id, required ProtocolMethod method, required Object params, int version = protocolVersion})", () => {
      writer.line(`final checkedVersion = ${validateExpression(service.protocolVersionType, "version", dartString(versionField), ir)};`);
      writer.line("if (checkedVersion != protocolVersion) throw ProtocolCodecException('protocolVersion', 'unsupported protocol version: $checkedVersion');");
      writer.line(`final checkedId = ${validateExpression(service.requestIdType, "id", dartString("id"), ir)};`);
      writer.line("_encodeRequestParams(method, params, 'params');");
      writer.line("return ProtocolRequestEnvelope._(version: checkedVersion, id: checkedId, method: method, params: params);");
    });
    writer.blank();
    writer.line("const ProtocolRequestEnvelope._({required this.version, required this.id, required this.method, required this.params});");
    writer.blank();
    writer.line("final int version;");
    writer.line("final RequestId id;");
    writer.line("final ProtocolMethod method;");
    writer.line("final Object params;");
    writer.blank();
    writer.block("factory ProtocolRequestEnvelope.fromJson(Object? value, {String path = 'ProtocolRequestEnvelope'})", () => {
      writer.line("final json = _object(value, path);");
      writer.line(`_expectKeys(json, const {${dartString(versionField)}, 'id', ${dartString(requestDiscriminator)}, 'params'}, path);`);
      writer.line(`final method = ProtocolMethod.fromJson(_required(json, ${dartString(requestDiscriminator)}, path), path: '\$path.${requestDiscriminator}');`);
      writer.line(`return ProtocolRequestEnvelope(id: ${decodeExpression(service.requestIdType, `_required(json, 'id', path)`, `'\$path.id'`, ir)}, method: method, params: _decodeRequestParams(method, _required(json, 'params', path), '\$path.params'), version: ${decodeExpression(service.protocolVersionType, `_required(json, ${dartString(versionField)}, path)`, `'\$path.${versionField}'`, ir)});`);
    });
    writer.blank();
    writer.line(`Map<String, Object?> toJson() => {${dartString(versionField)}: version, 'id': id, ${dartString(requestDiscriminator)}: method.toJson(), 'params': _encodeRequestParams(method, params, 'params')};`);
  });
  writer.blank();
  writer.block("sealed class ProtocolResponsePayload", () => writer.line("const ProtocolResponsePayload();"));
  writer.blank();
  writer.block("final class ProtocolSuccess extends ProtocolResponsePayload", () => {
    writer.line("const ProtocolSuccess(this.result);");
    writer.line("final Object result;");
  });
  writer.blank();
  writer.block("final class ProtocolFailure extends ProtocolResponsePayload", () => {
    writer.line("const ProtocolFailure(this.error);");
    writer.line("final ProtocolError error;");
  });
  writer.blank();
  writer.block("final class ProtocolResponseEnvelope", () => {
    writer.block("factory ProtocolResponseEnvelope({required RequestId id, required ProtocolMethod method, required ProtocolResponsePayload response, int version = protocolVersion})", () => {
      writer.line(`final checkedVersion = ${validateExpression(service.protocolVersionType, "version", dartString(versionField), ir)};`);
      writer.line("if (checkedVersion != protocolVersion) throw ProtocolCodecException('protocolVersion', 'unsupported protocol version: $checkedVersion');");
      writer.line(`final checkedId = ${validateExpression(service.requestIdType, "id", dartString("id"), ir)};`);
      writer.line("if (response is ProtocolSuccess) _encodeResponseResult(method, response.result, 'response.result');");
      writer.line("return ProtocolResponseEnvelope._(version: checkedVersion, id: checkedId, method: method, response: response);");
    });
    writer.blank();
    writer.line("const ProtocolResponseEnvelope._({required this.version, required this.id, required this.method, required this.response});");
    writer.blank();
    writer.line("final int version;");
    writer.line("final RequestId id;");
    writer.line("final ProtocolMethod method;");
    writer.line("final ProtocolResponsePayload response;");
    writer.blank();
    writer.block("factory ProtocolResponseEnvelope.fromJson(Object? value, {String path = 'ProtocolResponseEnvelope'})", () => {
      writer.line("final json = _object(value, path);");
      writer.line(`_expectKeys(json, const {${dartString(versionField)}, 'id', ${dartString(responseDiscriminator)}, 'response'}, path);`);
      writer.line(`final method = ProtocolMethod.fromJson(_required(json, ${dartString(responseDiscriminator)}, path), path: '\$path.${responseDiscriminator}');`);
      writer.line("final responseJson = _object(_required(json, 'response', path), '$path.response');");
      writer.line("final status = _string(_required(responseJson, 'status', '$path.response'), '$path.response.status');");
      writer.line("late final ProtocolResponsePayload response;");
      writer.block("if (status == 'ok')", () => {
        writer.line("_expectKeys(responseJson, const {'status', 'result'}, '$path.response');");
        writer.line("response = ProtocolSuccess(_decodeResponseResult(method, _required(responseJson, 'result', '$path.response'), '$path.response.result'));");
      });
      writer.block("else if (status == 'error')", () => {
        writer.line("_expectKeys(responseJson, const {'status', 'error'}, '$path.response');");
        writer.line("response = ProtocolFailure(ProtocolError.fromJson(_required(responseJson, 'error', '$path.response'), path: '$path.response.error'));");
      });
      writer.block("else", () => writer.line("throw ProtocolCodecException('$path.response.status', 'unknown response status: $status');"));
      writer.line(`return ProtocolResponseEnvelope(id: ${decodeExpression(service.requestIdType, `_required(json, 'id', path)`, `'\$path.id'`, ir)}, method: method, response: response, version: ${decodeExpression(service.protocolVersionType, `_required(json, ${dartString(versionField)}, path)`, `'\$path.${versionField}'`, ir)});`);
    });
    writer.blank();
    writer.block("Map<String, Object?> toJson()", () => {
      writer.line("final responseJson = switch (response) {");
      writer.line("  ProtocolSuccess(:final result) => <String, Object?>{'status': 'ok', 'result': _encodeResponseResult(method, result, 'response.result')},");
      writer.line("  ProtocolFailure(:final error) => <String, Object?>{'status': 'error', 'error': error.toJson()},");
      writer.line("};");
      writer.line(`return {${dartString(versionField)}: version, 'id': id, ${dartString(responseDiscriminator)}: method.toJson(), 'response': responseJson};`);
    });
  });
  writer.blank();
  writer.block("final class ProtocolEventEnvelope", () => {
    writer.block("factory ProtocolEventEnvelope({required EventCursor eventCursor, required ProtocolEventName event, required Object payload, int version = protocolVersion})", () => {
      writer.line(`final checkedVersion = ${validateExpression(service.protocolVersionType, "version", dartString(versionField), ir)};`);
      writer.line("if (checkedVersion != protocolVersion) throw ProtocolCodecException('protocolVersion', 'unsupported protocol version: $checkedVersion');");
      writer.line(`final checkedCursor = ${validateExpression(service.eventCursorType, "eventCursor", dartString(cursorField), ir)};`);
      writer.line("_encodeEventPayload(event, payload, 'payload');");
      writer.line("return ProtocolEventEnvelope._(version: checkedVersion, eventCursor: checkedCursor, event: event, payload: payload);");
    });
    writer.blank();
    writer.line("const ProtocolEventEnvelope._({required this.version, required this.eventCursor, required this.event, required this.payload});");
    writer.blank();
    writer.line("final int version;");
    writer.line("final EventCursor eventCursor;");
    writer.line("final ProtocolEventName event;");
    writer.line("final Object payload;");
    writer.blank();
    writer.block("factory ProtocolEventEnvelope.fromJson(Object? value, {String path = 'ProtocolEventEnvelope'})", () => {
      writer.line("final json = _object(value, path);");
      writer.line(`_expectKeys(json, const {${dartString(versionField)}, ${dartString(cursorField)}, ${dartString(eventDiscriminator)}, 'payload'}, path);`);
      writer.line(`final event = ProtocolEventName.fromJson(_required(json, ${dartString(eventDiscriminator)}, path), path: '\$path.${eventDiscriminator}');`);
      writer.line(`return ProtocolEventEnvelope(eventCursor: ${decodeExpression(service.eventCursorType, `_required(json, ${dartString(cursorField)}, path)`, `'\$path.${cursorField}'`, ir)}, event: event, payload: _decodeEventPayload(event, _required(json, 'payload', path), '\$path.payload'), version: ${decodeExpression(service.protocolVersionType, `_required(json, ${dartString(versionField)}, path)`, `'\$path.${versionField}'`, ir)});`);
    });
    writer.blank();
    writer.line(`Map<String, Object?> toJson() => {${dartString(versionField)}: version, ${dartString(cursorField)}: eventCursor, ${dartString(eventDiscriminator)}: event.toJson(), 'payload': _encodeEventPayload(event, payload, 'payload')};`);
  });
  writer.blank();
  writer.line("ProtocolRequestEnvelope decodeProtocolRequest(String source) => ProtocolRequestEnvelope.fromJson(jsonDecode(source));");
  writer.line("String encodeProtocolRequest(ProtocolRequestEnvelope value) => jsonEncode(value.toJson());");
  writer.line("ProtocolResponseEnvelope decodeProtocolResponse(String source) => ProtocolResponseEnvelope.fromJson(jsonDecode(source));");
  writer.line("String encodeProtocolResponse(ProtocolResponseEnvelope value) => jsonEncode(value.toJson());");
  writer.line("ProtocolEventEnvelope decodeProtocolEvent(String source) => ProtocolEventEnvelope.fromJson(jsonDecode(source));");
  writer.line("String encodeProtocolEvent(ProtocolEventEnvelope value) => jsonEncode(value.toJson());");
  writer.blank();
  writer.block("abstract interface class ProtocolTransport", () => {
    writer.line("Future<Object?> request(Map<String, Object?> request);");
  });
  writer.blank();
  writer.block("final class ProtocolRemoteException implements Exception", () => {
    writer.line("const ProtocolRemoteException(this.error);");
    writer.line("final ProtocolError error;");
    writer.line("@override");
    writer.line("String toString() => 'ProtocolRemoteException(${error.code}): ${error.message}';");
  });
  writer.blank();
  writer.block("final class ProtocolClient", () => {
    writer.line("const ProtocolClient(this.transport, {required this.requestIdFactory});");
    writer.blank();
    writer.line("final ProtocolTransport transport;");
    writer.line("final RequestId Function() requestIdFactory;");
    writer.blank();
    writer.block("Future<TResponse> _request<TResponse>(ProtocolMethod method, Object request) async", () => {
      writer.line("final id = requestIdFactory();");
      writer.line("final envelope = ProtocolRequestEnvelope(id: id, method: method, params: request);");
      writer.line("final response = ProtocolResponseEnvelope.fromJson(await transport.request(envelope.toJson()));");
      writer.line("if (response.id != id) throw ProtocolCodecException('id', 'response id does not match request id');");
      writer.line("if (response.method != method) throw ProtocolCodecException('method', 'response method does not match request method');");
      writer.line("return switch (response.response) {");
      writer.line("  ProtocolSuccess(:final result) when result is TResponse => result as TResponse,");
      writer.line("  ProtocolSuccess() => throw ProtocolCodecException('response.result', 'response result has the wrong generated type'),");
      writer.line("  ProtocolFailure(:final error) => throw ProtocolRemoteException(error),");
      writer.line("};");
    });
    for (const method of service.methods) {
      writer.blank();
      writer.line(`Future<${method.responseType.name}> ${camelCase(method.name)}(${method.requestType.name} request) => _request<${method.responseType.name}>(ProtocolMethod.${camelCase(method.name)}, request);`);
    }
  });
}

export function generateDart(record, _model, ir) {
  const pkg = packageIr(ir, record.packageConfig.id);
  for (const definition of pkg.definitions.filter((value) => value.kind === "union")) {
    assert(
      definition.discriminator,
      `${pkg.id}.${definition.name} is an untagged or ambiguous oneOf; Dart requires one common required singleton-enum discriminator`,
    );
  }
  const sourceFiles = [record.packageConfig.manifest, record.packageConfig.schema]
    .map((path) => `protocol/${path}`)
    .join(" and ");
  const writer = new DartWriter();
  writer.line(`// @generated by tools/protocol-codegen/generate.mjs from ${sourceFiles}.`);
  writer.line("// DO NOT EDIT MANUALLY.");
  writer.line("// ignore_for_file: unused_element, unnecessary_import");
  writer.blank();
  writer.line("import 'dart:convert';");
  for (const dependencyId of pkg.dependencies) {
    const dependency = _model.recordsById.get(dependencyId);
    const output = dependency?.packageConfig.outputs.dart;
    assert(output, `${pkg.id} needs Dart output from ${dependencyId}`);
    const segments = output.split("/");
    const libIndex = segments.lastIndexOf("lib");
    assert(libIndex > 0, `${dependencyId} Dart output must be inside lib`);
    const packageName = segments[libIndex - 1].replaceAll("-", "_");
    writer.line(`import 'package:${packageName}/${packageName}.dart';`);
  }
  writer.blank();
  emitCodecHelpers(writer, !pkg.dependencies.includes("core-v1"));
  writer.blank();
  if (!pkg.service) {
    const schemaVersionConstant = `${camelCase(pkg.id.replace(/-v\d+$/, ""))}SchemaVersion`;
    writer.line(`const int ${schemaVersionConstant} = ${pkg.version};`);
    writer.blank();
  }
  const parents = unionParents(pkg);
  for (const definition of pkg.definitions) {
    if (definition.kind === "alias") emitAlias(writer, definition, ir);
    else if (definition.kind === "enum") emitEnum(writer, definition);
    else if (definition.kind === "union") emitUnion(writer, definition);
    else if (definition.kind === "object") emitObject(writer, definition, parents.get(definition.name), ir);
    else fail(`unsupported Dart definition kind ${definition.kind}`);
    writer.blank();
  }
  if (pkg.service) emitProtocolMetadata(writer, pkg, ir);
  return writer.toString();
}

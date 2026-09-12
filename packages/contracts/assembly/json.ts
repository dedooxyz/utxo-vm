// Minimal, deterministic, escape-aware JSON parser for AssemblyScript
// Adheres to strict UTXO-VM determinism constraints (no floating point, no time leaks).

export enum JsonType {
  Null,
  Bool,
  Number,
  String,
  Array,
  Object,
}

export class JsonValue {
  type: JsonType;
  strVal: string;
  intVal: i64;
  boolVal: bool;
  arrVal: Array<JsonValue>;
  objVal: Map<string, JsonValue>;

  constructor(type: JsonType) {
    this.type = type;
    this.strVal = "";
    this.intVal = 0;
    this.boolVal = false;
    this.arrVal = new Array<JsonValue>();
    this.objVal = new Map<string, JsonValue>();
  }

  static fromString(val: string): JsonValue {
    let v = new JsonValue(JsonType.String);
    v.strVal = val;
    return v;
  }

  static fromInt(val: i64): JsonValue {
    let v = new JsonValue(JsonType.Number);
    v.intVal = val;
    return v;
  }

  static fromBool(val: bool): JsonValue {
    let v = new JsonValue(JsonType.Bool);
    v.boolVal = val;
    return v;
  }

  static fromNull(): JsonValue {
    return new JsonValue(JsonType.Null);
  }

  static fromObject(obj: Map<string, JsonValue>): JsonValue {
    let v = new JsonValue(JsonType.Object);
    v.objVal = obj;
    return v;
  }

  static fromArray(arr: Array<JsonValue>): JsonValue {
    let v = new JsonValue(JsonType.Array);
    v.arrVal = arr;
    return v;
  }

  has(key: string): bool {
    if (this.type != JsonType.Object) return false;
    return this.objVal.has(key);
  }

  get(key: string): JsonValue | null {
    if (this.type != JsonType.Object) return null;
    if (!this.objVal.has(key)) return null;
    return this.objVal.get(key);
  }

  getString(key: string, defaultVal: string = ""): string {
    let v = this.get(key);
    if (v == null) return defaultVal;
    if (v.type == JsonType.String) return v.strVal;
    return defaultVal;
  }

  getInt(key: string, defaultVal: i64 = 0): i64 {
    let v = this.get(key);
    if (v == null) return defaultVal;
    if (v.type == JsonType.Number) return v.intVal;
    // Allow quoted numbers like "1000000"
    if (v.type == JsonType.String) {
      let s = v.strVal;
      let start = 0;
      while (start < s.length && isWhitespaceChar(s.charCodeAt(start))) {
        start++;
      }
      let end = s.length;
      while (end > start && isWhitespaceChar(s.charCodeAt(end - 1))) {
        end--;
      }
      let trimmed = s.substring(start, end);
      if (trimmed.length > 0) {
        return I64.parseInt(trimmed);
      }
    }
    return defaultVal;
  }

  getBool(key: string, defaultVal: bool = false): bool {
    let v = this.get(key);
    if (v == null) return defaultVal;
    if (v.type == JsonType.Bool) return v.boolVal;
    if (v.type == JsonType.String) {
      if (v.strVal == "true") return true;
      if (v.strVal == "false") return false;
    }
    return defaultVal;
  }
}

function isWhitespaceChar(c: i32): bool {
  return c == 0x20 || c == 0x09 || c == 0x0A || c == 0x0D;
}

export class JsonParser {
  private str: string;
  private len: i32;
  private pos: i32;
  hasError: bool;
  errorMsg: string;

  constructor(json: string) {
    this.str = json;
    this.len = json.length;
    this.pos = 0;
    this.hasError = false;
    this.errorMsg = "";
  }

  fail(msg: string): void {
    if (!this.hasError) {
      this.hasError = true;
      this.errorMsg = msg + " at pos " + this.pos.toString();
    }
  }

  private skipWhitespace(): void {
    while (this.pos < this.len) {
      let c = this.str.charCodeAt(this.pos);
      if (isWhitespaceChar(c)) {
        this.pos++;
      } else {
        break;
      }
    }
  }

  parse(): JsonValue | null {
    this.skipWhitespace();
    if (this.pos >= this.len) {
      this.fail("Empty JSON input");
      return null;
    }
    let val = this.parseValue();
    if (this.hasError || val == null) return null;
    this.skipWhitespace();
    if (this.pos < this.len) {
      this.fail("Unexpected trailing content");
      return null;
    }
    return val;
  }

  private parseValue(): JsonValue | null {
    this.skipWhitespace();
    if (this.pos >= this.len) {
      this.fail("Unexpected EOF while parsing value");
      return null;
    }
    let c = this.str.charCodeAt(this.pos);
    if (c == 0x7B) { // '{'
      return this.parseObject();
    } else if (c == 0x5B) { // '['
      return this.parseArray();
    } else if (c == 0x22) { // '"'
      let s = this.parseString();
      if (this.hasError) return null;
      return JsonValue.fromString(s);
    } else if (c == 0x74 || c == 0x66) { // 't' or 'f'
      return this.parseBool();
    } else if (c == 0x6E) { // 'n'
      return this.parseNull();
    } else if (c == 0x2D || (c >= 0x30 && c <= 0x39)) { // '-' or '0'-'9'
      return this.parseNumber();
    } else {
      this.fail("Unexpected token character: " + String.fromCharCode(c));
      return null;
    }
  }

  private parseObject(): JsonValue | null {
    this.pos++; // consume '{'
    let map = new Map<string, JsonValue>();
    this.skipWhitespace();
    if (this.pos < this.len && this.str.charCodeAt(this.pos) == 0x7D) { // '}'
      this.pos++;
      return JsonValue.fromObject(map);
    }

    while (this.pos < this.len) {
      this.skipWhitespace();
      if (this.pos >= this.len || this.str.charCodeAt(this.pos) != 0x22) {
        this.fail("Expected string key in object");
        return null;
      }
      let key = this.parseString();
      if (this.hasError) return null;

      this.skipWhitespace();
      if (this.pos >= this.len || this.str.charCodeAt(this.pos) != 0x3A) { // ':'
        this.fail("Expected ':' after object key");
        return null;
      }
      this.pos++; // consume ':'

      let val = this.parseValue();
      if (this.hasError || val == null) return null;
      map.set(key, val);

      this.skipWhitespace();
      if (this.pos >= this.len) {
        this.fail("Unclosed object");
        return null;
      }
      let nextChar = this.str.charCodeAt(this.pos);
      if (nextChar == 0x7D) { // '}'
        this.pos++;
        return JsonValue.fromObject(map);
      } else if (nextChar == 0x2C) { // ','
        this.pos++;
      } else {
        this.fail("Expected ',' or '}' in object");
        return null;
      }
    }
    this.fail("Unclosed object at EOF");
    return null;
  }

  private parseArray(): JsonValue | null {
    this.pos++; // consume '['
    let arr = new Array<JsonValue>();
    this.skipWhitespace();
    if (this.pos < this.len && this.str.charCodeAt(this.pos) == 0x5D) { // ']'
      this.pos++;
      return JsonValue.fromArray(arr);
    }

    while (this.pos < this.len) {
      let val = this.parseValue();
      if (this.hasError || val == null) return null;
      arr.push(val);

      this.skipWhitespace();
      if (this.pos >= this.len) {
        this.fail("Unclosed array");
        return null;
      }
      let nextChar = this.str.charCodeAt(this.pos);
      if (nextChar == 0x5D) { // ']'
        this.pos++;
        return JsonValue.fromArray(arr);
      } else if (nextChar == 0x2C) { // ','
        this.pos++;
      } else {
        this.fail("Expected ',' or ']' in array");
        return null;
      }
    }
    this.fail("Unclosed array at EOF");
    return null;
  }

  private parseString(): string {
    this.pos++; // consume starting '"'
    let sb: string = "";
    let start = this.pos;
    while (this.pos < this.len) {
      let c = this.str.charCodeAt(this.pos);
      if (c == 0x22) { // ending '"'
        sb += this.str.substring(start, this.pos);
        this.pos++;
        return sb;
      } else if (c == 0x5C) { // '\' escape
        sb += this.str.substring(start, this.pos);
        this.pos++;
        if (this.pos >= this.len) {
          this.fail("Unexpected EOF in string escape");
          return "";
        }
        let esc = this.str.charCodeAt(this.pos);
        if (esc == 0x22) { // '\"'
          sb += "\"";
          this.pos++;
        } else if (esc == 0x5C) { // '\\'
          sb += "\\";
          this.pos++;
        } else if (esc == 0x2F) { // '\/'
          sb += "/";
          this.pos++;
        } else if (esc == 0x62) { // '\b'
          sb += "\b";
          this.pos++;
        } else if (esc == 0x66) { // '\f'
          sb += "\x0C";
          this.pos++;
        } else if (esc == 0x6E) { // '\n'
          sb += "\n";
          this.pos++;
        } else if (esc == 0x72) { // '\r'
          sb += "\r";
          this.pos++;
        } else if (esc == 0x74) { // '\t'
          sb += "\t";
          this.pos++;
        } else if (esc == 0x75) { // '\uXXXX'
          this.pos++;
          if (this.pos + 4 > this.len) {
            this.fail("Incomplete \\u hex escape in string");
            return "";
          }
          let hex = this.str.substring(this.pos, this.pos + 4);
          let code = I32.parseInt(hex, 16);
          sb += String.fromCharCode(code);
          this.pos += 4;
        } else {
          this.fail("Invalid escape character: \\" + String.fromCharCode(esc));
          return "";
        }
        start = this.pos;
      } else if (c < 0x20) {
        this.fail("Unescaped control character in string: " + c.toString());
        return "";
      } else {
        this.pos++;
      }
    }
    this.fail("Unclosed string literal");
    return "";
  }

  private parseBool(): JsonValue | null {
    if (this.pos + 4 <= this.len && this.str.substring(this.pos, this.pos + 4) == "true") {
      this.pos += 4;
      return JsonValue.fromBool(true);
    } else if (this.pos + 5 <= this.len && this.str.substring(this.pos, this.pos + 5) == "false") {
      this.pos += 5;
      return JsonValue.fromBool(false);
    } else {
      this.fail("Invalid boolean literal");
      return null;
    }
  }

  private parseNull(): JsonValue | null {
    if (this.pos + 4 <= this.len && this.str.substring(this.pos, this.pos + 4) == "null") {
      this.pos += 4;
      return JsonValue.fromNull();
    } else {
      this.fail("Invalid null literal");
      return null;
    }
  }

  private parseNumber(): JsonValue | null {
    let start = this.pos;
    if (this.pos < this.len && this.str.charCodeAt(this.pos) == 0x2D) { // '-'
      this.pos++;
    }
    let hasDigits = false;
    while (this.pos < this.len) {
      let c = this.str.charCodeAt(this.pos);
      if (c >= 0x30 && c <= 0x39) {
        hasDigits = true;
        this.pos++;
      } else {
        break;
      }
    }
    if (!hasDigits) {
      this.fail("Invalid number: missing digits");
      return null;
    }
    let numStr = this.str.substring(start, this.pos);
    let intVal = I64.parseInt(numStr);
    return JsonValue.fromInt(intVal);
  }
}

export function escapeJsonString(s: string): string {
  let res = "";
  for (let i = 0; i < s.length; i++) {
    let c = s.charCodeAt(i);
    if (c == 0x22) { // '"'
      res += "\\\"";
    } else if (c == 0x5C) { // '\'
      res += "\\\\";
    } else if (c == 0x0A) { // '\n'
      res += "\\n";
    } else if (c == 0x0D) { // '\r'
      res += "\\r";
    } else if (c == 0x09) { // '\t'
      res += "\\t";
    } else {
      res += String.fromCharCode(c);
    }
  }
  return res;
}

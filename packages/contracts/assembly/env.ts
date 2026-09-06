@external("env", "host_get_caller")
export declare function host_get_caller(out_ptr: usize): i32;

@external("env", "host_get_satoshis")
export declare function host_get_satoshis(): u64;

@external("env", "host_get_seal")
export declare function host_get_seal(out_ptr: usize): i32;

@external("env", "host_emit_event")
export declare function host_emit_event(topic_ptr: usize, data_ptr: usize, len: i32): void;

@external("env", "host_create_object")
export declare function host_create_object(code_hash_ptr: usize, state_ptr: usize, satoshis: u64): i32;

@external("env", "host_stealth_settle")
export declare function host_stealth_settle(stealth_addr_ptr: usize, satoshis: u64): i32;

@external("env", "host_mweb_peg_out")
export declare function host_mweb_peg_out(stealth_addr_ptr: usize, satoshis: u64): i32;

export class HostContext {
  static getCaller(): string {
    let buf = new ArrayBuffer(128);
    let ptr = changetype<usize>(buf);
    let len = host_get_caller(ptr);
    if (len <= 0) return "";
    return String.UTF8.decode(buf.slice(0, len), true);
  }

  static getSatoshis(): u64 {
    return host_get_satoshis();
  }

  static getSeal(): string {
    let buf = new ArrayBuffer(128);
    let ptr = changetype<usize>(buf);
    let len = host_get_seal(ptr);
    if (len <= 0) return "";
    return String.UTF8.decode(buf.slice(0, len), true);
  }

  static emitEvent(topic: string, data: string): void {
    let topicBuf = String.UTF8.encode(topic);
    let dataBuf = String.UTF8.encode(data);
    host_emit_event(
      changetype<usize>(topicBuf),
      changetype<usize>(dataBuf),
      dataBuf.byteLength
    );
  }

  static createObject(codeHash: string, initialState: string, satoshis: u64): i32 {
    let codeHashBuf = String.UTF8.encode(codeHash);
    let stateBuf = String.UTF8.encode(initialState);
    return host_create_object(
      changetype<usize>(codeHashBuf),
      changetype<usize>(stateBuf),
      satoshis
    );
  }

  static stealthSettle(stealthAddress: string, satoshis: u64): bool {
    let addrBuf = String.UTF8.encode(stealthAddress);
    let res = host_stealth_settle(changetype<usize>(addrBuf), satoshis);
    return res == 1;
  }

  static mwebPegOut(stealthAddress: string, satoshis: u64): bool {
    let addrBuf = String.UTF8.encode(stealthAddress);
    let res = host_mweb_peg_out(changetype<usize>(addrBuf), satoshis);
    return res == 1;
  }
}

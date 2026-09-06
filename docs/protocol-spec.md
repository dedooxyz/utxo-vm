# JKC-VM Wire Protocol & Inscription Specification

## 1. Data Carrier Formats

JKC-VM supports two standard on-chain data carriers:

### 1.1 Taproot / Witness Script Inscription Envelope (Primary)
```text
OP_FALSE
OP_IF
  OP_PUSH "jkcvm"               // 5 bytes tag
  OP_PUSH 0x01                  // 1 byte protocol version
  OP_PUSH "application/wasm"    // MIME content type (or application/cbor)
  OP_PUSH <CONTENT_BYTES>       // Raw WASM bytecode or calldata payload
OP_ENDIF
```

### 1.2 OP_RETURN Payload (Compact Method Calls)
For small function invocations (<= 80 bytes), calldata can be encoded in standard `OP_RETURN`:
```text
OP_RETURN <0x6a> <"JKC"> <method_id: 4 bytes> <calldata_bytes>
```

---

## 2. State Transition Function

Let a Smart Object be $O = (ID, CodeHash, State, Satoshis, Owner)$.
A valid transaction $Tx$ satisfies:

1. **Input Proof**: $Tx$ spends the UTXO matching $O.Seal = (TxID_{prev}, Vout_{prev})$.
2. **Signature Proof**: $Tx$ is signed by the private key corresponding to $O.Owner$ (or satisfies MWEB Stealth verification).
3. **VM Determinism**:
   $$(State', Outputs', Events) = \text{Execute}(Code, State, Method, Args, Context)$$
4. **Conservation of Value**:
   $$\sum Satoshis_{in} = \sum Satoshis_{out} + Fee$$

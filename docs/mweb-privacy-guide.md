# Mimblewimble Extension Block (MWEB) Privacy Guide for JKC-VM

## 1. Background

Junkcoin implements the Mimblewimble Extension Block (MWEB). MWEB provides:
1. **Confidential Transactions**: Values are blinded using Pedersen Commitments $C = r \cdot G + v \cdot H$.
2. **Stealth Addresses**: Transactions send funds to one-time ephemeral public keys derived via Diffie-Hellman:
   $$P = H(r \cdot K) \cdot G + K$$

## 2. Privacy Patterns in JKC-VM

### Pattern A: Shielded Liquidity Vault (Peg-In / Peg-Out)
1. Alice transfers public JKC to MWEB (Peg-In).
2. Inside MWEB, Alice transfers to a fresh stealth address.
3. Alice Pegs-Out directly to a JKC-VM Smart Contract UTXO. The contract receives clean funds without on-chain ancestry.

### Pattern B: Stealth Smart Object Ownership
Instead of setting `_owner` to a static public key, set `_owner` to an MWEB stealth key. Only the holder of the MWEB viewing/spend key can identify and interact with the contract.

### Pattern C: Dark Pool Orderbook Settlement (PSOB + MWEB)
Partially Signed Order Book orders are matched peer-to-peer. The settlement transaction deposits the seller's proceeds directly into an MWEB stealth output, making trade volume private.

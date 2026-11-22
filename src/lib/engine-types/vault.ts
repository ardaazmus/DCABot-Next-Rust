/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */

import type { VaultEnvironment } from "./core";

export interface VaultKeyPermissions {
  read_only: boolean;
  can_withdraw: boolean;
  spot_trade: boolean;
  contract_trade: boolean;
  ips?: string | null;
  note?: string;
}

export interface VaultKey {
  id: string;
  venue: string;
  environment: VaultEnvironment;
  label: string;
  permissions: VaultKeyPermissions;
  maskedKey: string;
  /** P4-3: OKX passphrase kayıtlı mı (değer ASLA dışarı çıkmaz). */
  hasPassphrase?: boolean | null;
  /** Optional operator note stored with the key. */
  note?: string | null;
  /** ISO-8601 timestamp when the key was added to the vault. */
  createdAt?: string | null;
}

export type VaultKeyResponse = VaultKey;

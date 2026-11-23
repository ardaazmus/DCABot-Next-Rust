# Yerel PAPER veritabanı yedekleme ve geri yükleme

Bu kılavuz yalnız `.local-runtime/paper.sqlite3` yerel PAPER veritabanı içindir. Gerçek borsa hesabı durumu bu dosyaya geri yüklenmez. Veritabanı kasa kayıtları şifreli biçimde içerebilir; yedekleri proje dışına göndermeyin.

## Tutarlı yedek oluşturma

Node.js 22.5 veya üstü gerekir (`node:sqlite` ve `VACUUM INTO`). Komut, WAL içindeki verilerle tutarlı tek dosyalı snapshot oluşturur ve `PRAGMA quick_check` sonucu `ok` değilse başarısız olur.

PowerShell'i proje kökünde açın:

```powershell
$env:DCA_DB_PATH = Join-Path (Get-Location) '.local-runtime/paper.sqlite3'
$env:DCA_BACKUP_DIR = Join-Path (Get-Location) '.local-runtime/manual-backups'
node scripts/backup-db.mjs
Remove-Item Env:DCA_DB_PATH, Env:DCA_BACKUP_DIR -ErrorAction SilentlyContinue
```

Çıktıdaki `Snapshot created` yolu kullanılabilir yedeği gösterir. Betik yedeğin bütünlüğünü doğrular ve seçilen yedek dizinindeki yedi günden eski `dcabot_backup_*.sqlite3` dosyalarını temizler.

## Snapshot doğrulama

Yedeği geri yüklemeden önce aynı SQLite bütünlük kontrolünü tekrar çalıştırın. `$candidate` değerini gerçek snapshot yoluyla değiştirin:

```powershell
$candidate = (Resolve-Path -LiteralPath '.local-runtime/manual-backups/dcabot_backup_YYYYMMDD_HHMMSSmmm_PID.sqlite3').Path
$env:DCA_VERIFY_DB = $candidate
@'
import { DatabaseSync } from "node:sqlite";
const db = new DatabaseSync(process.env.DCA_VERIFY_DB, { readOnly: true });
try {
  const result = db.prepare("PRAGMA quick_check").get()?.quick_check;
  console.log(`quick_check=${result}`);
  if (result !== "ok") process.exitCode = 1;
} finally {
  db.close();
}
'@ | node --input-type=module
Remove-Item Env:DCA_VERIFY_DB -ErrorAction SilentlyContinue
```

Yalnız `quick_check=ok` çıktısı veren dosyayı geri yükleyin.

## Yerel PAPER veritabanını geri yükleme

Önce mevcut dosyanın tutarlı bir kurtarma snapshot'ını alın. Bu adım, geri yükleme yanlış dosya seçilirse dönüş yolunu korur.

```powershell
$env:DCA_DB_PATH = Join-Path (Get-Location) '.local-runtime/paper.sqlite3'
$env:DCA_BACKUP_DIR = Join-Path (Get-Location) '.local-runtime/pre-restore-backup'
node scripts/backup-db.mjs
Remove-Item Env:DCA_DB_PATH, Env:DCA_BACKUP_DIR -ErrorAction SilentlyContinue
npm run local:stop
```

Motor ve UI durduktan sonra hedef ve snapshot yollarını açıkça doğrulayın. `paper.sqlite3` yerine başka bir dosyaya kopyalama yapmayın:

```powershell
$root = (Get-Location).Path
$runtimeRoot = (Resolve-Path -LiteralPath (Join-Path $root '.local-runtime')).Path
$target = Join-Path $root '.local-runtime/paper.sqlite3'
$candidate = (Resolve-Path -LiteralPath '.local-runtime/manual-backups/dcabot_backup_YYYYMMDD_HHMMSSmmm_PID.sqlite3').Path
if (-not $candidate.StartsWith($runtimeRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Snapshot .local-runtime dışında.' }
if (-not $target.StartsWith($runtimeRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Hedef .local-runtime dışında.' }
Remove-Item -LiteralPath "$target-wal", "$target-shm" -ErrorAction SilentlyContinue
Copy-Item -LiteralPath $candidate -Destination $target -Force
```

Servisler durmadan WAL/SHM yan dosyalarını silmeyin. Dosya kopyalandıktan sonra `Snapshot doğrulama` bölümündeki komutla hedefi de doğrulayın. Ardından:

```powershell
npm run local:start
```

`http://127.0.0.1:3002/api/health` yanıtında `status=ok`, `readiness.db=ok` ve `executionEnvironments.PAPER=INTERNAL_PAPER` olduğunu; UI'nin `http://127.0.0.1:3000/` adresinde açıldığını doğrulayın. Botların sayısı ve PAPER durumları seçilen snapshot'ın zamanına göre değişebilir.

Geri yükleme başarısız olursa servisleri durdurun ve `.local-runtime/pre-restore-backup` içindeki doğrulanmış snapshot'ı aynı adımlarla geri yükleyin. Yedekler `.local-runtime` altında kalmalı; API, kasa ve oturum sırlarıyla birlikte paylaşılmamalıdır.

## Veri saklama notu

Yedi günlük politika yalnız yedek snapshot'larını kapsar. Bot, fill, olay ve kasa kayıtları için otomatik silme politikası tanımlı değildir; bu kayıtların saklama süresi ve arşiv gereksinimi ürün/operasyon kararıdır. Bu nedenle canlı DB kayıtlarına otomatik retention veya silme eklenmemiştir.

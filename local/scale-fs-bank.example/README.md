# Scale-FS live bank samples

Copy to a working bank (gitignored):

```powershell
Copy-Item -Recurse .\local\scale-fs-bank.example .\local\scale-fs-bank
```

```bash
cp -R ./local/scale-fs-bank.example ./local/scale-fs-bank
```

Then run `scripts/scale-fs-regression.ps1` / `.sh`.

Corpus files are generated at eval time (not stored in the bank JSON).

# i18n glossary (en → ar → fr)

One word per idea, everywhere in the app. Add a row before you introduce a new term; reviewers check
`ar.json` and `fr.json` against this table.

Audience: non-technical users first (Arabic speakers in Mauritania and the wider Arab world). Arabic is
plain Modern Standard Arabic, not a word-for-word calque of the English.

## Rules

- **Keep** = leave the name in Latin script, in every locale: product, tool and file-type names
  (Nezzel in paths, SponsorBlock, yt-dlp, ffmpeg, aria2, MP4, MP3, MKV, M4A, Opus, VPN, API, HTTP,
  cookies.txt, socks5/http URLs). In Arabic, wrap Latin runs in `<bdi>` when the UI builds the sentence.
- **Translate** = use the Arabic/French word in the table, never the English.
- **Both** = Arabic word followed by the Latin name in parentheses on first use in a screen, then the
  Arabic word alone (only Proxy so far).
- No trailing colons on labels (`الحفظ في`, not `الحفظ في:`). Colons inside a sentence are fine.
- Digits are Latin (0-9) in every locale, including Arabic prose (`من 1 إلى 4`).
- Keycaps (⌘ ⌥ ⇧ ⌃, Ctrl, Alt, Shift) are never translated and always render LTR.
- Avoid jargon in help text: say what happens to the file, not how (no "container", "re-encode",
  "post-processing", "extractor", "clipboard monitoring" without a plain explanation).
- Banned calques: `معاملات المستخرج` (extractor arguments), `محاكاة` (impersonation), `الحاوية`
  (container), `إعادة الترميز` (re-encode), `علبة النظام` (system tray), `مرشّحات` (filters).

## Terms

| en | ar | fr | rule |
|---|---|---|---|
| Nezzel (in a sentence) | نزّل | Nezzel | translate in ar prose; Latin in the folder path `Downloads / Nezzel` (it is the real folder name) |
| download (verb) | تنزيل / نزّل | télécharger | translate |
| Downloads (folder, list) | التنزيلات | Téléchargements | translate |
| link | رابط | lien | translate |
| paste | لصق | coller | translate |
| copied link | الرابط المنسوخ | lien copié | translate; prefer this over "clipboard" in UI copy |
| copied-link card (`input.watchClipboard`, Settings and the "…" menu) | بطاقة منبثقة للروابط المنسوخة / بطاقة الروابط المنسوخة | carte pour les liens copiés | translate; one name in every place; distinct from "Suggest copied links" (`اقتراح`, fills the link field) |
| clipboard | الحافظة | presse-papiers | translate; only when unavoidable |
| Settings / General / Advanced | الإعدادات / عام / متقدم | Réglages / Général / Avancé | translate |
| login(s) | تسجيل الدخول / تسجيلات الدخول | connexion(s) | translate |
| browser | المتصفح | navigateur | translate |
| cookies.txt | cookies.txt | cookies.txt | keep |
| Save to | الحفظ في | Enregistrer dans | translate |
| folder | مجلد | dossier | translate |
| Change… | تغيير… | Modifier… | translate |
| file type (MP4, MP3 setting) | نوع الملف | type de fichier | translate; never "container"/`الحاوية` |
| format (one quality + file type choice in the sheet, capture bar) | صيغة | format | translate |
| quality | الجودة | qualité | translate |
| video / audio / photo | فيديو / صوت / صورة | vidéo / audio / photo | translate |
| MP4, MP3, MKV, M4A, Opus | as is | as is | keep |
| thumbnail | الصورة المصغّرة | miniature | translate |
| cover image | صورة الغلاف | pochette | translate |
| title and artist (metadata) | العنوان واسم الفنان | titre et artiste | translate; never "metadata"/`البيانات الوصفية` in labels |
| subtitles | الترجمة | sous-titres | translate |
| playlist | قائمة التشغيل | playlist | translate (fr keeps "playlist", common usage) |
| playlist filters / link filters | تصفية قائمة التشغيل / تصفية الروابط | filtres de playlist / filtres de liens | translate; `تصفية`, never `مرشّحات` |
| convert (re-encode) | تحويل | convertir | translate; "slow" in parentheses where it matters |
| copy without converting (remux) | نسخ دون تحويل | copier sans convertir | translate |
| arguments (ffmpeg) | خيارات | arguments | translate in ar; keep "ffmpeg" Latin |
| extractor arguments | خيارات yt-dlp الإضافية (extractor-args) | arguments d’extracteur (yt-dlp) | developer only; keep `extractor-args` in parentheses |
| proxy | بروكسي (Proxy) → بروكسي | proxy | both (first use), then ar alone |
| VPN | VPN | VPN | keep |
| look like a browser (impersonation) | الظهور كمتصفح | se présenter comme un navigateur | translate; never `محاكاة` |
| downloads at the same time | عدد التنزيلات في وقت واحد | téléchargements simultanés | translate |
| pieces (per download) | أجزاء | morceaux | translate |
| aria2 | aria2 | aria2 | keep |
| SponsorBlock | SponsorBlock | SponsorBlock | keep |
| sponsor segments | مقاطع الإعلانات | passages sponsorisés | translate |
| segments | المقاطع | segments | translate |
| chapters | فصول | chapitres | translate |
| precise cuts | قصّ دقيق | coupes précises | translate |
| server | خادم | serveur | translate |
| YouTube (in our copy) | يوتيوب | YouTube | translate in ar prose; site names from metadata stay as the site sends them |
| keyboard shortcut(s) | اختصار / اختصارات لوحة المفاتيح | raccourci(s) clavier | translate |
| Quick download | التنزيل السريع | Téléchargement rapide | translate |
| Reset to default | إعادة إلى الافتراضي | Rétablir par défaut | translate |
| Default: ⌘⇧D | الافتراضي: ⌘⇧D | Par défaut : ⌘⇧D | translate; keys LTR |
| On / Off (state) | مفعّل / معطّل | Activé / Désactivé | translate |
| recommended | موصى به | recommandé | translate |
| Show more / Show less | عرض المزيد / عرض أقل | Afficher plus / Afficher moins | translate |
| notifications | الإشعارات | notifications | translate |
| update (verb) | تحديث | mettre à jour | translate |
| download tools (yt-dlp, ffmpeg) | أدوات التنزيل | outils de téléchargement | translate |
| keep running in the background (tray) | يعمل في الخلفية | continuer en arrière-plan | translate; never `علبة النظام` |
| theme / appearance / language | السمة / المظهر / اللغة | thème / apparence / langue | translate |
| Developer options | خيارات المطوّرين | Options développeur | translate |
| Technical details | تفاصيل تقنية | Détails techniques | translate |
| logs | السجلات | journaux | translate; only inside Technical details |
| Copy details | نسخ التفاصيل | Copier les détails | translate |
| Retry | إعادة المحاولة | Réessayer | translate |
| Ready / Downloading / Paused / Done / Failed | جاهز / قيد التنزيل / متوقف مؤقتًا / اكتمل / فشل | Prêt / Téléchargement / En pause / Terminé / Échec | translate |
| waiting (not started) | ينتظر / بانتظار التنزيل | en attente | translate |
| speed units (MB/s, KB/s) | MB/s | Mo/s | keep in ar; fr uses Mo/Ko |
| Open / Show in folder | فتح / إظهار في المجلد | Ouvrir / Afficher dans le dossier | translate |

// Interface translations. Keys are English sentences' identifiers; every
// language falls back to English for any missing key.

export const LANGS = [
  ["en", "English"], ["es", "Español"], ["fr", "Français"], ["de", "Deutsch"],
  ["pt", "Português"], ["ar", "العربية"], ["ur", "اردو"], ["zh", "中文"],
] as const;
export type Lang = (typeof LANGS)[number][0];
const RTL = new Set<Lang>(["ar", "ur"]);

type Dict = Record<string, string>;

const en: Dict = {
  tagline: "Image metadata and Content Credentials, handled on this device.",
  add: "Add images", addFolder: "Add folder", clearList: "Clear list", language: "Language",
  dropTitle: "Drop JPEG, PNG or WebP images or folders", dropOr: "Or", dropChoose: "choose files",
  dropLocal: "Files are read locally and never uploaded.",
  remove: "Remove", keep: "Keep", cleanN: "Clean {n} images", clean1: "Clean 1 image", working: "Working…",
  saveAll: "Save all", saveOne: "Save cleaned image", saveAgain: "Saved as {name} · Save again",
  exportReport: "Export JSON report", exportAll: "Export report for all",
  "t.exif": "EXIF", "h.exif": "Camera, dates, GPS location, serial numbers",
  "t.xmp": "XMP", "h.xmp": "Editing history, descriptions, AI labels written as XMP",
  "t.iptc": "IPTC", "h.iptc": "Captions, credits, keywords",
  "t.comments": "Comments and text", "h.comments": "Comments and text fields",
  "t.other": "Other data", "h.other": "Maker data, unknown blocks, data after the image",
  "t.embedded": "Motion photo and extra images", "h.embedded": "Video in motion photos, depth and HDR gain maps",
  "t.icc": "Colour profile", "h.icc": "Keep it for accurate colours",
  "t.c2pa": "Content Credentials", "h.c2pa": "Signed C2PA provenance record",
  "t.keep_orientation": "Orientation", "h.keep_orientation": "Rotated photos stay upright",
  "t.keep_dates": "Capture dates", "h.keep_dates": "Date taken, modified and time zone",
  "t.keep_resolution": "Print resolution", "h.keep_resolution": "DPI used when printing",
  "t.keep_time": "File date", "h.keep_time": "Saved files keep the original modification time",
  reading: "Reading {i} of {n}…", added: "{n} files added.", added1: "1 file added.", noneFound: "No supported images found.",
  cleaning: "Cleaning {i} of {n}…", cleanedAll: "{n} cleaned and verified. Save to keep the results.",
  cleanedSome: "{ok} cleaned and verified, {bad} could not be verified and will not be saved.",
  saved: "Saved {name}.", savedTo: "Saved {n} images to {dir}.", skipped: "{n} unverified results were not saved.",
  reportSaved: "Report saved as {name}.", couldNotOpen: "Could not open the files: {e}", couldNotSave: "Could not save: {e}",
  original: "Original", cleaned: "Cleaned", identicalPixels: "identical pixels", noPreview: "No preview",
  verifiedAfter: "Verified after cleaning", verificationFailed: "Verification failed",
  found: "Found in this file", noMetadata: "No metadata found.", removedList: "Removed: {list}.", keptList: "Kept: {list}.",
  nothingRemoved: "nothing selected was present", noMeta: "no metadata", smaller: "{size} smaller.",
  imageIdentical: "Image data identical to the original", imageDiffers: "Image data differs from the original",
  allRemoved: "Everything selected was removed", stillPresent: "Still present: {list}",
  transparencyKept: "transparency kept", animationKept: "animation kept",
  credRemoved: "Content Credentials removed from this file", credValid: "Content Credentials kept and still pass validation",
  credBroken: "Content Credentials kept but now fail validation", credWasInvalid: "Content Credentials kept; they did not pass validation before cleaning either",
  notSaved: "This result did not pass verification, so it cannot be saved. The original file is unchanged.",
  compare: "Before and after", field: "Field", before: "Before", after: "After", removedTag: "removed",
  compareSummary: "{removed} fields removed, {kept} kept.",
  details: "All metadata", filter: "Filter fields", fieldsN: "{n} fields", noFields: "No readable fields.",
  credTitle: "Content Credentials", credNone: "None embedded in this file.",
  "cred.trusted": "Trusted signature", "credx.trusted": "The signature is intact and the signer is on a trust list.",
  "cred.valid": "Signature intact", "credx.valid": "The credentials match this file. The signer is not checked against a trust list, so their identity is not confirmed.",
  "cred.invalid": "Validation failed", "credx.invalid": "The credentials do not match this file or the signature is broken. Do not rely on them.",
  "cred.unreadable": "Could not be read", "credx.unreadable": "The credentials are present but damaged or unsupported.",
  credAi: "These credentials declare AI-generated content.", signedBy: "Signed by", signedAt: "Signed at",
  madeWith: "Made with", title: "Title", actions: "Actions", algorithm: "Algorithm", manifests: "Manifests",
  credMessages: "{n} validation messages",
  c2paWarning: "Content Credentials cover the whole file. Removing any other metadata while keeping them makes them fail validation. To keep them valid, turn off every other option.",
  "k.exif": "EXIF", "k.xmp": "XMP", "k.iptc": "IPTC", "k.icc": "Colour profile", "k.c2pa": "Content Credentials",
  "k.comment": "Comments and text", "k.embedded": "Motion photo and extra images", "k.other": "Other data",
  chipFound: "{n} found", chipVerified: "Verified", chipSaved: "Saved", chipNotVerified: "Not verified",
  chipUnsupported: "Unsupported", chipFailed: "Failed",
  "err.unsupported": "Not a supported image. Nexonic Clear handles JPEG, PNG and WebP.",
  "err.too_large": "The file is larger than 256 MB.",
  "err.malformed": "The file is damaged or not a valid image, so it was not changed.",
  "err.io": "The file could not be read or written. Check that it still exists and that you have permission.",
  "err.unverified": "This result did not pass verification and cannot be saved.",
  "err.gone": "This result is no longer available. Clean the image again.",
  errDetail: "Details", errorsN: "{n} files could not be processed.",
  "m.c2pa_broken": "Content Credentials were kept, but they no longer match the file because other metadata changed. Viewers will report them as invalid.",
  "m.c2pa_removed": "Embedded Content Credentials were removed from this file. Copies elsewhere, cloud records and platform labels are not affected.",
  "m.image_changed": "The image data changed.",
  "m.structure_changed": "The format, dimensions, transparency or animation changed.",
  "m.still_present": "Some selected metadata is still present.",
  "m.embedded_changed": "Embedded motion-photo or extra-image content changed.",
  "m.motion_xmp_removed": "The motion photo or extra images were kept, but XMP was removed. Some viewers use XMP to find them and may now show only the still image.",
  "m.embedded_removed": "The embedded motion-photo video or extra images were removed.",
};

const es: Dict = {
  tagline: "Metadatos de imagen y Content Credentials, gestionados en este dispositivo.",
  add: "Añadir imágenes", addFolder: "Añadir carpeta", clearList: "Vaciar lista", language: "Idioma",
  dropTitle: "Suelta imágenes JPEG, PNG o WebP, o carpetas", dropOr: "O", dropChoose: "elige archivos",
  dropLocal: "Los archivos se leen localmente y nunca se suben.",
  remove: "Eliminar", keep: "Conservar", cleanN: "Limpiar {n} imágenes", clean1: "Limpiar 1 imagen", working: "Procesando…",
  saveAll: "Guardar todo", saveOne: "Guardar imagen limpia", saveAgain: "Guardada como {name} · Guardar de nuevo",
  exportReport: "Exportar informe JSON", exportAll: "Exportar informe de todo",
  "t.other": "Otros datos", "t.comments": "Comentarios y texto", "t.embedded": "Foto en movimiento e imágenes extra",
  "t.icc": "Perfil de color", "t.keep_orientation": "Orientación", "t.keep_dates": "Fechas de captura",
  "t.keep_resolution": "Resolución de impresión", "t.keep_time": "Fecha del archivo",
  "h.exif": "Cámara, fechas, ubicación GPS, números de serie", "h.icc": "Consérvalo para colores precisos",
  "h.keep_orientation": "Las fotos giradas se ven derechas", "h.keep_dates": "Fecha de captura, modificación y zona horaria",
  "h.keep_resolution": "PPP usados al imprimir", "h.keep_time": "Los archivos guardados conservan la fecha original",
  reading: "Leyendo {i} de {n}…", noneFound: "No se encontraron imágenes compatibles.", cleaning: "Limpiando {i} de {n}…",
  cleanedAll: "{n} limpiadas y verificadas. Guarda para conservar los resultados.",
  saved: "Guardado {name}.", savedTo: "{n} imágenes guardadas en {dir}.",
  original: "Original", cleaned: "Limpia", identicalPixels: "píxeles idénticos",
  verifiedAfter: "Verificada tras la limpieza", verificationFailed: "La verificación falló",
  found: "Encontrado en este archivo", noMetadata: "No se encontraron metadatos.",
  imageIdentical: "Datos de imagen idénticos al original", allRemoved: "Se eliminó todo lo seleccionado",
  compare: "Antes y después", field: "Campo", before: "Antes", after: "Después", removedTag: "eliminado",
  details: "Todos los metadatos", filter: "Filtrar campos", credTitle: "Content Credentials",
  credNone: "No hay ninguna incrustada en este archivo.", "cred.valid": "Firma intacta", "cred.invalid": "La validación falló",
  credAi: "Estas credenciales declaran contenido generado por IA.",
  chipVerified: "Verificada", chipSaved: "Guardada", chipNotVerified: "No verificada", chipFailed: "Error",
  "err.unsupported": "No es una imagen compatible. Nexonic Clear admite JPEG, PNG y WebP.",
  "err.malformed": "El archivo está dañado o no es una imagen válida, así que no se modificó.",
  notSaved: "Este resultado no superó la verificación, así que no se puede guardar. El original no cambia.",
};

const fr: Dict = {
  tagline: "Métadonnées d’image et Content Credentials, traitées sur cet appareil.",
  add: "Ajouter des images", addFolder: "Ajouter un dossier", clearList: "Vider la liste", language: "Langue",
  dropTitle: "Déposez des images JPEG, PNG ou WebP, ou des dossiers", dropOr: "Ou", dropChoose: "choisissez des fichiers",
  dropLocal: "Les fichiers sont lus localement et jamais envoyés.",
  remove: "Supprimer", keep: "Conserver", cleanN: "Nettoyer {n} images", clean1: "Nettoyer 1 image", working: "Traitement…",
  saveAll: "Tout enregistrer", saveOne: "Enregistrer l’image nettoyée", exportReport: "Exporter le rapport JSON",
  exportAll: "Exporter le rapport complet",
  "t.other": "Autres données", "t.comments": "Commentaires et texte", "t.embedded": "Photo animée et images supplémentaires",
  "t.icc": "Profil colorimétrique", "t.keep_orientation": "Orientation", "t.keep_dates": "Dates de prise de vue",
  "t.keep_resolution": "Résolution d’impression", "t.keep_time": "Date du fichier",
  reading: "Lecture de {i} sur {n}…", cleaning: "Nettoyage de {i} sur {n}…",
  original: "Original", cleaned: "Nettoyée", identicalPixels: "pixels identiques",
  verifiedAfter: "Vérifiée après nettoyage", verificationFailed: "Échec de la vérification",
  found: "Trouvé dans ce fichier", compare: "Avant et après", field: "Champ", before: "Avant", after: "Après",
  removedTag: "supprimé", details: "Toutes les métadonnées", filter: "Filtrer les champs",
  credNone: "Aucune intégrée dans ce fichier.", "cred.valid": "Signature intacte", "cred.invalid": "Échec de la validation",
  chipVerified: "Vérifiée", chipSaved: "Enregistrée", chipNotVerified: "Non vérifiée", chipFailed: "Échec",
  "err.malformed": "Le fichier est endommagé ou n’est pas une image valide ; il n’a pas été modifié.",
};

const de: Dict = {
  tagline: "Bild-Metadaten und Content Credentials, direkt auf diesem Gerät verarbeitet.",
  add: "Bilder hinzufügen", addFolder: "Ordner hinzufügen", clearList: "Liste leeren", language: "Sprache",
  dropTitle: "JPEG-, PNG- oder WebP-Bilder oder Ordner hier ablegen", dropOr: "Oder", dropChoose: "Dateien auswählen",
  dropLocal: "Dateien werden lokal gelesen und nie hochgeladen.",
  remove: "Entfernen", keep: "Behalten", cleanN: "{n} Bilder bereinigen", clean1: "1 Bild bereinigen", working: "Wird bearbeitet…",
  saveAll: "Alle speichern", saveOne: "Bereinigtes Bild speichern", exportReport: "JSON-Bericht exportieren",
  exportAll: "Bericht für alle exportieren",
  "t.other": "Sonstige Daten", "t.comments": "Kommentare und Text", "t.embedded": "Bewegtfoto und Zusatzbilder",
  "t.icc": "Farbprofil", "t.keep_orientation": "Ausrichtung", "t.keep_dates": "Aufnahmedaten",
  "t.keep_resolution": "Druckauflösung", "t.keep_time": "Dateidatum",
  reading: "Lese {i} von {n}…", cleaning: "Bereinige {i} von {n}…",
  original: "Original", cleaned: "Bereinigt", identicalPixels: "identische Pixel",
  verifiedAfter: "Nach dem Bereinigen geprüft", verificationFailed: "Prüfung fehlgeschlagen",
  found: "In dieser Datei gefunden", compare: "Vorher und nachher", field: "Feld", before: "Vorher", after: "Nachher",
  removedTag: "entfernt", details: "Alle Metadaten", filter: "Felder filtern",
  credNone: "In dieser Datei nicht vorhanden.", "cred.valid": "Signatur intakt", "cred.invalid": "Prüfung fehlgeschlagen",
  chipVerified: "Geprüft", chipSaved: "Gespeichert", chipNotVerified: "Nicht geprüft", chipFailed: "Fehler",
  "err.malformed": "Die Datei ist beschädigt oder kein gültiges Bild und wurde nicht verändert.",
};

const pt: Dict = {
  tagline: "Metadados de imagem e Content Credentials, tratados neste dispositivo.",
  add: "Adicionar imagens", addFolder: "Adicionar pasta", clearList: "Limpar lista", language: "Idioma",
  dropTitle: "Solte imagens JPEG, PNG ou WebP, ou pastas", dropOr: "Ou", dropChoose: "escolha arquivos",
  dropLocal: "Os arquivos são lidos localmente e nunca enviados.",
  remove: "Remover", keep: "Manter", cleanN: "Limpar {n} imagens", clean1: "Limpar 1 imagem", working: "Processando…",
  saveAll: "Salvar tudo", saveOne: "Salvar imagem limpa", exportReport: "Exportar relatório JSON",
  exportAll: "Exportar relatório de tudo",
  "t.other": "Outros dados", "t.comments": "Comentários e texto", "t.embedded": "Foto em movimento e imagens extras",
  "t.icc": "Perfil de cor", "t.keep_orientation": "Orientação", "t.keep_dates": "Datas de captura",
  "t.keep_resolution": "Resolução de impressão", "t.keep_time": "Data do arquivo",
  reading: "Lendo {i} de {n}…", cleaning: "Limpando {i} de {n}…",
  original: "Original", cleaned: "Limpa", identicalPixels: "pixels idênticos",
  verifiedAfter: "Verificada após a limpeza", verificationFailed: "A verificação falhou",
  found: "Encontrado neste arquivo", compare: "Antes e depois", field: "Campo", before: "Antes", after: "Depois",
  removedTag: "removido", details: "Todos os metadados", filter: "Filtrar campos",
  credNone: "Nenhuma incorporada neste arquivo.", "cred.valid": "Assinatura intacta", "cred.invalid": "A validação falhou",
  chipVerified: "Verificada", chipSaved: "Salva", chipNotVerified: "Não verificada", chipFailed: "Falhou",
  "err.malformed": "O arquivo está danificado ou não é uma imagem válida, então não foi alterado.",
};

const ar: Dict = {
  tagline: "بيانات الصور الوصفية وبيانات الاعتماد للمحتوى، تُعالَج على هذا الجهاز.",
  add: "إضافة صور", addFolder: "إضافة مجلد", clearList: "مسح القائمة", language: "اللغة",
  dropTitle: "أفلت صور JPEG أو PNG أو WebP أو مجلدات", dropOr: "أو", dropChoose: "اختر ملفات",
  dropLocal: "تُقرأ الملفات محليًا ولا تُرفع أبدًا.",
  remove: "إزالة", keep: "إبقاء", cleanN: "تنظيف {n} صور", clean1: "تنظيف صورة واحدة", working: "جارٍ العمل…",
  saveAll: "حفظ الكل", saveOne: "حفظ الصورة المنظّفة", exportReport: "تصدير تقرير JSON", exportAll: "تصدير تقرير للكل",
  "t.other": "بيانات أخرى", "t.comments": "تعليقات ونص", "t.embedded": "صورة متحركة وصور إضافية",
  "t.icc": "ملف تعريف الألوان", "t.keep_orientation": "الاتجاه", "t.keep_dates": "تواريخ الالتقاط",
  "t.keep_resolution": "دقة الطباعة", "t.keep_time": "تاريخ الملف",
  reading: "قراءة {i} من {n}…", cleaning: "تنظيف {i} من {n}…",
  original: "الأصل", cleaned: "بعد التنظيف", identicalPixels: "بكسلات مطابقة",
  verifiedAfter: "تم التحقق بعد التنظيف", verificationFailed: "فشل التحقق",
  found: "وُجد في هذا الملف", compare: "قبل وبعد", field: "الحقل", before: "قبل", after: "بعد",
  removedTag: "أُزيل", details: "كل البيانات الوصفية", filter: "تصفية الحقول",
  credNone: "لا توجد بيانات مضمّنة في هذا الملف.", "cred.valid": "التوقيع سليم", "cred.invalid": "فشل التحقق",
  chipVerified: "تم التحقق", chipSaved: "محفوظة", chipNotVerified: "لم يتم التحقق", chipFailed: "فشل",
  "err.malformed": "الملف تالف أو ليس صورة صالحة، لذلك لم يُغيَّر.",
};

const ur: Dict = {
  tagline: "تصویر کا میٹا ڈیٹا اور Content Credentials، اسی ڈیوائس پر۔",
  add: "تصاویر شامل کریں", addFolder: "فولڈر شامل کریں", clearList: "فہرست صاف کریں", language: "زبان",
  dropTitle: "JPEG، PNG یا WebP تصاویر یا فولڈر یہاں چھوڑیں", dropOr: "یا", dropChoose: "فائلیں منتخب کریں",
  dropLocal: "فائلیں اسی ڈیوائس پر پڑھی جاتی ہیں اور کبھی اپ لوڈ نہیں ہوتیں۔",
  remove: "ہٹائیں", keep: "رکھیں", cleanN: "{n} تصاویر صاف کریں", clean1: "1 تصویر صاف کریں", working: "کام جاری ہے…",
  saveAll: "سب محفوظ کریں", saveOne: "صاف تصویر محفوظ کریں", exportReport: "JSON رپورٹ برآمد کریں", exportAll: "سب کی رپورٹ برآمد کریں",
  "t.other": "دیگر ڈیٹا", "t.comments": "تبصرے اور متن", "t.embedded": "موشن فوٹو اور اضافی تصاویر",
  "t.icc": "رنگ پروفائل", "t.keep_orientation": "رخ", "t.keep_dates": "تصویر کھینچنے کی تاریخیں",
  "t.keep_resolution": "پرنٹ ریزولوشن", "t.keep_time": "فائل کی تاریخ",
  reading: "{n} میں سے {i} پڑھی جا رہی ہے…", cleaning: "{n} میں سے {i} صاف ہو رہی ہے…",
  original: "اصل", cleaned: "صاف شدہ", identicalPixels: "ہوبہو پکسلز",
  verifiedAfter: "صفائی کے بعد تصدیق شدہ", verificationFailed: "تصدیق ناکام",
  found: "اس فائل میں ملا", compare: "پہلے اور بعد", field: "فیلڈ", before: "پہلے", after: "بعد",
  removedTag: "ہٹا دیا", details: "تمام میٹا ڈیٹا", filter: "فیلڈز فلٹر کریں",
  credNone: "اس فائل میں موجود نہیں۔", "cred.valid": "دستخط درست ہیں", "cred.invalid": "تصدیق ناکام",
  chipVerified: "تصدیق شدہ", chipSaved: "محفوظ", chipNotVerified: "غیر تصدیق شدہ", chipFailed: "ناکام",
  "err.malformed": "فائل خراب ہے یا درست تصویر نہیں، اس لیے اسے تبدیل نہیں کیا گیا۔",
};

const zh: Dict = {
  tagline: "图像元数据和内容凭证，全部在本设备上处理。",
  add: "添加图片", addFolder: "添加文件夹", clearList: "清空列表", language: "语言",
  dropTitle: "拖入 JPEG、PNG、WebP 图片或文件夹", dropOr: "或", dropChoose: "选择文件",
  dropLocal: "文件只在本地读取，绝不上传。",
  remove: "移除", keep: "保留", cleanN: "清理 {n} 张图片", clean1: "清理 1 张图片", working: "处理中…",
  saveAll: "全部保存", saveOne: "保存清理后的图片", exportReport: "导出 JSON 报告", exportAll: "导出全部报告",
  "t.other": "其他数据", "t.comments": "注释和文本", "t.embedded": "动态照片和附加图像",
  "t.icc": "颜色配置文件", "t.keep_orientation": "方向", "t.keep_dates": "拍摄日期",
  "t.keep_resolution": "打印分辨率", "t.keep_time": "文件日期",
  reading: "正在读取第 {i}/{n} 个…", cleaning: "正在清理第 {i}/{n} 个…",
  original: "原图", cleaned: "清理后", identicalPixels: "像素完全相同",
  verifiedAfter: "清理后已验证", verificationFailed: "验证失败",
  found: "此文件中找到", compare: "前后对比", field: "字段", before: "之前", after: "之后",
  removedTag: "已移除", details: "全部元数据", filter: "筛选字段",
  credNone: "此文件中没有内嵌凭证。", "cred.valid": "签名完好", "cred.invalid": "验证失败",
  chipVerified: "已验证", chipSaved: "已保存", chipNotVerified: "未验证", chipFailed: "失败",
  "err.malformed": "文件已损坏或不是有效图片，因此未作更改。",
};

const DICTS: Record<Lang, Dict> = { en, es, fr, de, pt, ar, ur, zh };

let current: Lang = "en";

function detect(): Lang {
  try {
    const saved = localStorage.getItem("nexonic.clear.lang") as Lang | null;
    if (saved && saved in DICTS) return saved;
  } catch { /* storage unavailable */ }
  const nav = (navigator.language || "en").slice(0, 2).toLowerCase() as Lang;
  return nav in DICTS ? nav : "en";
}

export function setLang(lang: Lang) {
  current = lang in DICTS ? lang : "en";
  document.documentElement.lang = current;
  document.documentElement.dir = RTL.has(current) ? "rtl" : "ltr";
  try { localStorage.setItem("nexonic.clear.lang", current); } catch { /* ignore */ }
}

export function lang(): Lang {
  return current;
}

export function t(key: string, vars: Record<string, string | number> = {}): string {
  const s = DICTS[current][key] ?? en[key] ?? key;
  return s.replace(/\{(\w+)\}/g, (_, k) => String(vars[k] ?? ""));
}

setLang(detect());

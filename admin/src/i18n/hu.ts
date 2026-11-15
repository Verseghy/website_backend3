/**
 * The Hungarian message file: the single source of every user-visible string
 * in the admin (ADR 0021 §2).
 *
 * Why one file: the admin is Hungarian only (ADR 0021 §1). English exists only
 * as the fallback for a missing key, which is a defect that the completeness
 * checks catch (`hu.test.ts`, `key-usage.test.ts` and the `local/no-ui-literals`
 * lint rule).
 *
 * Provenance: the `ra.*` strings started from `ra-language-hungarian` 1.4.0
 * (MIT; its licence is `LICENSE.ra-language-hungarian` next to this file, and
 * `THIRD_PARTY.md` lists the copy). Every inherited string was reviewed like a
 * new one and most were rewritten. The 19 keys that pack lacks against
 * `ra-language-english` 5.15.5, and the three keys only shadcn-admin-kit uses,
 * were translated for backend3.
 *
 * Conventions every string here follows:
 * - Register: informal (tegező), the register of the public site's update
 *   prompt that ADR 0004 §5 reuses („Kérlek töltsd újra az oldalt!”).
 * - Typography: „…” quotes (»…« inside them), an en dash for ranges, and a
 *   singular noun after a numeral („3 elem”), so most count messages need one
 *   form. Polyglot treats `hu` with the two-form rule (one, other); a string
 *   uses `||||` only where the one-item sentence differs.
 * - Terminology: ADR 0021 §6. In particular „menü” alone is never used (it
 *   clashes with the canteen's dishes), and „Visszaállítás” is reserved for
 *   restoring from the trash, so "reset" is „alaphelyzetbe állítás”.
 * - Variables: only those ra-core and the kit pass (`%{name}`, `%{smart_count}`
 *   …). A test rejects placeholders that the English string does not have.
 *   An `errors.*` message uses only the names the server sends with its code:
 *   the envelope's `params`, or the `index` and `detail` of a rich-text
 *   finding (`ContentError`); `hu.test.ts` lists them per code.
 *
 * Namespaces (ADR 0021 §2, amended by ADR 0030): `ra.*` holds exactly the
 * keys of `ra-language-english` plus the three kit-only keys; `kit.*` holds the
 * other strings of the copied kit and shadcn/ui files, which upstream
 * hard-codes in English (ADR 0021 §3); `app.*` holds backend3's application
 * shell (title, footer, about page, update prompt, data-provider notice);
 * `dashboard.*` holds the dashboard; `auth.*` holds the sign-in screens and the
 * sign-out of ADR 0006; `recorder.*` holds the development-only clipboard
 * recorder (editor.md §13.1). ADR 0030 §3 says which namespace a new key
 * belongs to.
 * Vertical slices add `resources.*`, `enums.*`, `errors.*` and their own
 * screens' namespaces in the same PR as the screen. A top-level
 * key that the namespace list of `key-usage.test.ts` (ADR 0030 §2's table)
 * lacks fails that test.
 *
 * `errors.*` has one message per stable error code of the admin API, filled
 * from the envelope's `params` (ADR 0021 §4). It is declared
 * `satisfies Record<ErrorCode, string>` against the ts-rs binding, so a code
 * the server adds without a Hungarian message fails `npm run typecheck`.
 */
import type { ErrorCode } from "../bindings/ErrorCode"

export const hu = {
  ra: {
    action: {
      add_filter: "Szűrő hozzáadása",
      add: "Hozzáadás",
      back: "Vissza",
      bulk_actions: "%{smart_count} elem kijelölve",
      cancel: "Mégse",
      clear_array_input: "Lista ürítése",
      clear_input_value: "Érték törlése",
      // Kit-only key (shadcn-admin-kit search-input); not in ra-language-english.
      clear_search: "Keresés törlése",
      clone: "Másolat készítése",
      confirm: "Megerősítés",
      create: "Új",
      create_item: "„%{item}” létrehozása",
      delete: "Törlés",
      edit: "Szerkesztés",
      export: "Exportálás",
      list: "Lista",
      refresh: "Frissítés",
      remove_filter: "Szűrő eltávolítása",
      remove_all_filters: "Összes szűrő eltávolítása",
      remove: "Eltávolítás",
      reset: "Alaphelyzetbe állítás",
      save: "Mentés",
      search: "Keresés",
      search_columns: "Oszlopok keresése",
      select_all: "Összes kijelölése",
      select_all_button: "Összes kijelölése",
      select_row: "Sor kijelölése",
      show: "Megtekintés",
      sort: "Rendezés",
      undo: "Visszavonás",
      unselect: "Kijelölés megszüntetése",
      expand: "Kibontás",
      close: "Bezárás",
      open_menu: "Navigáció megnyitása",
      close_menu: "Navigáció bezárása",
      update: "Módosítás",
      move_up: "Mozgatás felfelé",
      move_down: "Mozgatás lefelé",
      open: "Megnyitás",
      toggle_theme: "Világos vagy sötét mód",
      select_columns: "Oszlopok",
      update_application: "Alkalmazás újratöltése",
    },
    boolean: {
      true: "Igen",
      false: "Nem",
      // ra-core renders this for an empty boolean; a space keeps the cell blank.
      null: " ",
    },
    page: {
      create: "%{name} létrehozása",
      dashboard: "Irányítópult",
      edit: "%{name} szerkesztése: %{recordRepresentation}",
      error: "Hiba történt",
      list: "%{name}",
      loading: "Betöltés",
      not_found: "Nem található",
      show: "%{name}: %{recordRepresentation}",
      // `name` arrives capitalised („Bejegyzések”), so it opens the sentence.
      empty: "%{name}: még nincs egy sem.",
      invite: "Létrehozol egyet?",
      access_denied: "Hozzáférés megtagadva",
      authentication_error: "Hitelesítési hiba",
    },
    input: {
      file: {
        upload_several:
          "Húzd ide a feltöltendő fájlokat, vagy kattints a kiválasztásukhoz.",
        upload_single:
          "Húzd ide a feltöltendő fájlt, vagy kattints a kiválasztásához.",
      },
      image: {
        upload_several:
          "Húzd ide a feltöltendő képeket, vagy kattints a kiválasztásukhoz.",
        upload_single:
          "Húzd ide a feltöltendő képet, vagy kattints a kiválasztásához.",
      },
      references: {
        all_missing: "A hivatkozott adatok nem találhatók.",
        many_missing: "Legalább egy hivatkozott elem már nem érhető el.",
        single_missing: "A hivatkozott elem már nem érhető el.",
      },
      password: {
        toggle_visible: "Jelszó elrejtése",
        toggle_hidden: "Jelszó megjelenítése",
      },
    },
    message: {
      about: "Névjegy",
      access_denied: "Nincs jogosultságod az oldal megtekintéséhez.",
      are_you_sure: "Biztos vagy benne?",
      authentication_error:
        "A hitelesítési szerver hibát jelzett, ezért a belépési adataidat nem sikerült ellenőrizni.",
      auth_error: "Hiba történt a hitelesítési token ellenőrzésekor.",
      bulk_delete_content:
        "Biztosan törlöd a kijelölt elemet? |||| Biztosan törlöd a kijelölt %{smart_count} elemet?",
      bulk_delete_title: "%{name} törlése |||| %{smart_count} elem törlése",
      bulk_update_content:
        "Biztosan módosítod ezt az elemet: %{recordRepresentation}? |||| Biztosan módosítod a kijelölt %{smart_count} elemet?",
      bulk_update_title:
        "%{name} módosítása: %{recordRepresentation} |||| %{smart_count} elem módosítása",
      clear_array_input: "Biztosan kiüríted a teljes listát?",
      delete_content: "Biztosan törlöd ezt az elemet?",
      delete_title: "%{name} törlése: %{recordRepresentation}",
      details: "Részletek",
      error: "Kliensoldali hiba történt, a kérést nem sikerült teljesíteni.",
      invalid_form: "Az űrlap hibás. Ellenőrizd a jelzett mezőket!",
      loading: "Kis türelmet…",
      no: "Nem",
      not_found: "Rossz címet írtál be, vagy hibás linket követtél.",
      select_all_limit_reached:
        "Túl sok elem van ahhoz, hogy mindet kijelöld. Csak az első %{max} elem lett kijelölve.",
      unsaved_changes:
        "Néhány módosításod nincs elmentve. Biztosan elveted őket?",
      yes: "Igen",
      placeholder_data_warning:
        "Hálózati hiba: az adatok frissítése nem sikerült.",
    },
    navigation: {
      // Kit-only keys (shadcn-admin-kit breadcrumb); not in ra-language-english.
      breadcrumb_drawer_title: "Ugrás",
      breadcrumb_drawer_instructions: "Válaszd ki a megnyitandó oldalt.",
      clear_filters: "Szűrők törlése",
      no_filtered_results: "%{name}: a jelenlegi szűrőkkel nincs találat.",
      no_results: "%{name}: nincs találat.",
      no_more_results: "Nincs %{page}. oldal. Próbáld meg az előző oldalt.",
      page_out_of_boundaries: "Nincs %{page}. oldal.",
      page_out_from_end: "Az utolsó oldalnál tovább nem lehet lapozni.",
      page_out_from_begin: "Az első oldal elé nem lehet lapozni.",
      page_range_info: "%{offsetBegin}–%{offsetEnd} / %{total}",
      partial_page_range_info:
        "%{offsetBegin}–%{offsetEnd} / több mint %{offsetEnd}",
      current_page: "%{page}. oldal",
      page: "%{page}. oldal megnyitása",
      first: "Ugrás az első oldalra",
      last: "Ugrás az utolsó oldalra",
      next: "Ugrás a következő oldalra",
      previous: "Ugrás az előző oldalra",
      page_rows_per_page: "Sorok oldalanként:",
      skip_nav: "Ugrás a tartalomra",
    },
    sort: {
      sort_by: "Rendezés: %{field_lower_first}, %{order}",
      ASC: "növekvő",
      DESC: "csökkenő",
    },
    auth: {
      auth_check_error: "A folytatáshoz jelentkezz be.",
      user_menu: "Profil",
      username: "Felhasználónév",
      password: "Jelszó",
      email: "E-mail-cím",
      sign_in: "Bejelentkezés",
      sign_in_error: "A bejelentkezés nem sikerült. Próbáld újra!",
      logout: "Kijelentkezés",
    },
    notification: {
      updated: "Módosítva |||| %{smart_count} elem módosítva",
      created: "Létrehozva",
      deleted: "Törölve |||| %{smart_count} elem törölve",
      bad_item: "Hibás elem",
      item_doesnt_exist: "Az elem nem létezik",
      http_error: "Hiba a szerverrel való kommunikációban",
      data_provider_error:
        "Adatszolgáltatói hiba. A részleteket lásd a böngésző konzolján.",
      i18n_error: "A kiválasztott nyelv fordításai nem tölthetők be.",
      canceled: "Művelet megszakítva",
      logged_out: "A munkamenet lejárt, jelentkezz be újra.",
      not_authorized: "Nincs jogosultságod ennek az elérésére.",
      application_update_available: "Új verzió érhető el.",
      offline: "Nincs hálózati kapcsolat. Az adatokat nem sikerült lekérni.",
    },
    validation: {
      required: "Kötelező",
      minLength: "Legalább %{min} karakter legyen",
      maxLength: "Legfeljebb %{max} karakter lehet",
      minValue: "Legalább %{min} legyen",
      maxValue: "Legfeljebb %{max} lehet",
      number: "Számnak kell lennie",
      email: "Érvényes e-mail-címnek kell lennie",
      oneOf: "Ezek egyike lehet: %{options}",
      regex:
        "Meghatározott formátumúnak kell lennie (reguláris kifejezés): %{pattern}",
      unique: "Egyedinek kell lennie",
    },
    saved_queries: {
      label: "Mentett lekérdezések",
      query_name: "Lekérdezés neve",
      new_label: "Jelenlegi lekérdezés mentése…",
      new_dialog_title: "Jelenlegi lekérdezés mentése ezen a néven",
      remove_label: "Mentett lekérdezés törlése",
      remove_label_with_name: "„%{name}” lekérdezés törlése",
      remove_dialog_title: "Törlöd a mentett lekérdezést?",
      remove_message:
        "Biztosan törlöd ezt az elemet a mentett lekérdezések közül?",
      help: "Szűrd a listát, és mentsd el a lekérdezést későbbre",
    },
    guesser: {
      empty: {
        title: "Nincs megjeleníthető adat",
        message: "Ellenőrizd az adatszolgáltatót",
      },
    },
    configurable: {
      customize: "Testreszabás",
      configureMode: "Az oldal testreszabása",
      inspector: {
        title: "Elemvizsgáló",
        content: "Vidd az egeret a felület elemei fölé a beállításukhoz",
        reset: "Beállítások alaphelyzetbe állítása",
        hideAll: "Összes elrejtése",
        showAll: "Összes megjelenítése",
      },
      Datagrid: {
        title: "Táblázat",
        unlabeled: "%{column}. névtelen oszlop",
      },
      SimpleForm: {
        title: "Űrlap",
        unlabeled: "%{input}. névtelen mező",
      },
      SimpleList: {
        title: "Lista",
        primaryText: "Elsődleges szöveg",
        secondaryText: "Másodlagos szöveg",
        tertiaryText: "Harmadlagos szöveg",
      },
    },
  },
  app: {
    // Orthography: a personal name and a common noun are hyphenated.
    title: "Verseghy-admin",
    // ADR 0004 §5: the public site's wording, reused verbatim.
    update: {
      available: "Frissítés elérhető. Kérlek töltsd újra az oldalt!",
      reload: "Újratöltés",
    },
    footer: {
      about: "Névjegy",
      source_code: "Forráskód",
      source_code_title:
        "A futó verzió forráskódja a GitHubon (AGPL-3.0 licenc)",
    },
    about: {
      intro: "A Verseghy Ferenc Gimnázium honlapjának szerkesztőfelülete.",
      licence:
        "Az admin szabad szoftver: a GNU Affero General Public License 3.0 (AGPL-3.0-only) feltételei szerint használható, módosítható és terjeszthető.",
      source: "A futó verzió forráskódja",
      commit: "Verzió (commit): %{commit}",
      local_build:
        "Helyi build: nincs commithez kötve, ezért a forráskód a master ágra mutat.",
    },
    data_provider: {
      // No placeholders: ra-core shows a data-provider error by translating its
      // `message` with only `_` as an option (useListController `onError`).
      not_available: "Ez a funkció még nem érhető el.",
    },
  },
  // Strings of the copied shadcn-admin-kit and shadcn/ui files that ra-core
  // has no key for (the `kit.*` row of ADR 0030 §2). The kit hard-codes them
  // in English; ADR 0021 §3 routes them through this file (the edits are
  // listed in src/components/admin/LOCAL_CHANGES.md and
  // src/components/ui/LOCAL_CHANGES.md).
  kit: {
    autocomplete: {
      no_match: "Nincs találat.",
    },
    breadcrumb: {
      label: "Elérési út",
      more: "Továbbiak",
      show_path: "Teljes elérési út megjelenítése",
    },
    data_table: {
      empty: "Nincs megjeleníthető sor.",
    },
    pagination: {
      label: "Lapozás",
      more: "További oldalak",
      previous: "Előző",
      next: "Következő",
    },
    ready: {
      title: "Az admin elindult",
      message: "Még nincs beállított erőforrás.",
    },
    sidebar: {
      title: "Oldalsáv",
      description: "Navigáció mobilnézetben",
      toggle: "Oldalsáv megjelenítése vagy elrejtése",
    },
    theme: {
      light: "Világos",
      dark: "Sötét",
      system: "Rendszerbeállítás szerint",
    },
  },
  dashboard: {
    welcome: "Üdv a Verseghy-adminban!",
    placeholder:
      "Az irányítópult a statisztikákkal és a teendőkkel egy későbbi kiadásban érkezik.",
  },
  // The sign-in screens and the sign-out of ADR 0006 (ADR 0021 §2's `auth.*`
  // row). A wrong address or password shows `errors.invalid_credentials`, the
  // one generic message of every failed sign-in (ADR 0006 §4).
  auth: {
    sign_in: {
      title: "Bejelentkezés",
      intro: "Jelentkezz be a szerkesztőfelület használatához.",
      email: "E-mail-cím",
      password: "Jelszó",
      submit: "Bejelentkezés",
      // A network failure, or an answer without a readable error envelope
      // (a server error without one, a proxy's page, an unknown step).
      failed: "A bejelentkezés most nem sikerült. Próbáld újra később!",
    },
    // The user menu's „Kijelentkezés” (`ra.auth.logout`), which ends every
    // session of the account on the server (ADR 0006 §2).
    sign_out: {
      // The server did not confirm the sign-out (a network failure, a
      // server error without an envelope): the session still holds, so the
      // user stays signed in and is told so.
      failed:
        "A kijelentkezés nem sikerült, továbbra is be vagy jelentkezve. Próbáld újra!",
    },
    // The in-place sign-in that opens over the page when the session has
    // ended (ADR 0006, "Follow-up work"); its inputs and its submit button
    // reuse the `auth.sign_in.*` labels.
    session_expired: {
      title: "Lejárt a munkamenet",
      intro:
        "Jelentkezz be újra a jelszavaddal. Az oldal nyitva marad: amit beírtál, megmarad, és a félbemaradt művelet a bejelentkezés után folytatódik.",
      leave_notice:
        "Ha kilépsz, a bejelentkezési oldalra jutsz, és a nem mentett módosítások elvesznek.",
      leave: "Kilépés",
    },
  },
  // The development-only clipboard recorder (editor.md §13.1). Its route is
  // left out of the default build (src/recorder/recorder-flag.ts); these
  // strings stay, because this file is one module.
  recorder: {
    title: "Vágólap-rögzítő",
    dev_only:
      "Fejlesztői eszköz a beillesztési tesztekhez: az éles admin nem tartalmazza.",
    intro:
      "Másold ki a tartalmat (például Wordből vagy Excelből), kattints a beillesztési mezőbe, és illeszd be a Ctrl+V billentyűkkel. A rögzítő egy JSON-fájlba menti a vágólap típusait, a HTML- és a szöveges tartalmat, valamint a fájlok típusát és méretét, majd letölti a fájlt. RTF-et és fájltartalmat nem ment.",
    synthetic:
      "Csak erre a célra írt, kitalált tartalmú dokumentumot rögzíts, valódi iskolai dokumentumot vagy személyes adatot soha.",
    scrub_notice:
      "A felhasználói mappák nevét (például C:\\Users\\… vagy /Users/…) a rögzítő kitakarja, a dokumentumban szereplő neveket viszont nem: a fájlt nézd át, mielőtt a tesztek közé kerül.",
    paste_area: "Beillesztési mező",
    paste_hint:
      "Kattints a lenti mezőbe, és illeszd be a vágólap tartalmát (Ctrl+V).",
    no_clipboard:
      "A böngésző nem adta át a vágólap tartalmát. Illeszd be újra a Ctrl+V billentyűkkel.",
    saved:
      "Letöltve: %{file}. Típusok: %{types}. Kitakart mappanevek: %{scrubbed}.",
  },
  // The rich-text editor (ADR 0013, editor.md §4.3 rows 8-10): the image's
  // controls in the editor, and the YouTube and Padlet insert dialogs.
  editor: {
    image: {
      // editor.md §4.3 row 8 names the badge's text.
      alt_missing: "Alt szöveg hiányzik",
      alt_edit: "Alternatív szöveg",
      alt_title: "A kép alternatív szövege",
      alt_description:
        "Írd le röviden, mi látható a képen. A képernyőolvasók ezt olvassák fel, és ez jelenik meg, ha a kép nem töltődik be.",
      alt_label: "Alternatív szöveg",
      sizes: "Képméret",
      size_preset: "%{width} px",
      uploading: "Feltöltés folyamatban…",
      upload_failed: "A kép feltöltése nem sikerült.",
    },
    youtube: {
      title: "YouTube-videó beillesztése",
      description:
        "Másold be a videó vagy a lejátszási lista címét, vagy a YouTube „Beágyazás” kódját.",
      input_label: "YouTube-cím vagy beágyazási kód",
      title_label: "A videó címe",
      title_hint:
        "A képernyőolvasók ezt olvassák fel. Ha üresen hagyod, „Videó” lesz.",
      // ADR 0043 §4 (D134) gives this notice verbatim.
      options_dropped:
        "A YouTube-videó néhány beállítása (például a vezérlők elrejtése vagy az automatikus lejátszás) nem marad meg, a videó a szokásos lejátszóval jelenik meg.",
      not_youtube:
        "Ez nem egy YouTube-videó vagy lejátszási lista címe, és nem is YouTube-beágyazási kód.",
      video_id:
        "Ebben a YouTube-címben nincs érvényes videó- vagy listaazonosító. Másold ki újra a címet a YouTube-ról!",
      list_type:
        "Ezt a YouTube-listát nem lehet beágyazni: csak egy videó vagy egy lejátszási lista illeszthető be.",
      not_top_level:
        "Videót csak önálló bekezdésbe lehet beágyazni, listába, táblázatba, idézetbe vagy címsorba nem, ezért a YouTube-cím szövegként maradt.",
      insert: "Beillesztés",
    },
    padlet: {
      title: "Padlet beillesztése",
      description:
        "Másold be a Padlet beágyazási címét, amely így kezdődik: https://padlet.com/embed/",
      input_label: "Padlet-cím",
      not_padlet:
        "Ez nem Padlet beágyazási cím. A címnek így kell kezdődnie: https://padlet.com/embed/",
      insert: "Beillesztés",
    },
  },
  errors: {
    not_found: "A keresett elem nem található.",
    invalid_credentials: "Hibás e-mail-cím vagy jelszó.",
    invalid_body:
      "A kérés formátuma hibás. Frissítsd az oldalt, és próbáld újra!",
    body_too_large: "Az elküldött adat túl nagy.",
    // A refused rich-text body (editor.md §7.6). Each finding is a
    // `ContentError` ({code, index, detail}); the editor fills these with its
    // `index` (the image's or embed's position) and its `detail` values.
    // - The four image and embed messages need `index`. It is null only for
    //   a finding the server could not trace back to the body, which cannot
    //   happen by construction; the editor then shows the finding without a
    //   position instead of filling these with a null `index`.
    // - `content_too_large` gets `actual` and `limit` in bytes, which the
    //   editor fills in as sizes („2 MB”, ADR 0021 §5), so the message has no
    //   unit of its own. Both numbers measure the same thing (`measured`:
    //   the submitted body, or what the sanitiser wrote from it), so the
    //   message never sets a limit against a size it does not apply to.
    image_alt_missing: "A(z) %{index}. képnek nincs alternatív szövege.",
    image_src_rejected:
      "A(z) %{index}. kép forrása nem engedélyezett. Képet a Kép gombbal tölthetsz fel, vagy választhatsz a médiatárból.",
    embed_rejected:
      "A(z) %{index}. beágyazás nem engedélyezett: csak YouTube- és Padlet-beágyazás használható.",
    iframe_content: "A(z) %{index}. beágyazás nem tartalmazhat szöveget.",
    content_too_large:
      "A tartalom túl nagy (%{actual}): legfeljebb %{limit} lehet.",
    content_too_deep:
      "A tartalomban túl mélyen vannak egymásba ágyazva az elemek (%{line}. sor): legfeljebb %{limit} szint lehet.",
    content_not_normalisable:
      "A szerver nem tudta egységes formára hozni a tartalmat. Kérlek, jelezd ezt a hibát a honlap üzemeltetőjének!",
    // A request without a valid admin session (ADR 0006 §1), one message for
    // every reason: sign in again.
    unauthorized: "A munkamenet lejárt vagy érvénytelen. Jelentkezz be újra!",
    // A signed-in account that the route's requirement refuses (ADR 0007
    // §5): the session stays valid, so the message does not ask the user to
    // sign in again.
    forbidden: "Nincs jogosultságod ehhez a művelethez.",
    // A write with the session cookie that did not come from the admin's own
    // address (ADR 0006 §3, ADR 0044): a page on another site or host, or a
    // browser extension that drops the Origin header. Nothing was done and
    // the session stays valid, so the message asks the user to work from the
    // admin's usual address instead of signing in again.
    origin_mismatch:
      "A szerver elutasította a kérést, mert nem a szerkesztőfelület saját címéről érkezett. Nyisd meg a szerkesztőfelületet a megszokott címén, és próbáld újra!",
    // Upload refusals (ADR 0015 §6 and §7, ADR 0033 §4): each says what
    // makes the file acceptable.
    image_not_allowed: "Csak JPEG, PNG, GIF vagy WebP kép tölthető fel.",
    image_type_mismatch:
      "A fájl kiterjesztése (.%{extension}) nem egyezik a tartalmával, amely %{format} kép. Mentsd el a képet a tartalmához illő kiterjesztéssel, majd töltsd fel újra!",
    image_corrupt:
      "A %{format} kép sérült, ezért nem tölthető fel. Nyisd meg egy képszerkesztőben, mentsd el újra, majd töltsd fel ismét!",
    image_too_many_pixels:
      "A kép felbontása túl nagy (%{width} × %{height} képpont). Legfeljebb %{max_megapixels} megapixeles képet tölthetsz fel: kicsinyítsd le, majd töltsd fel újra!",
    image_animation_too_large:
      "Az animáció túl hosszú vagy túl nagy: legfeljebb %{max_frames} képkockából és összesen %{max_megapixels} megapixelből állhat. Rövidítsd vagy kicsinyítsd le, majd töltsd fel újra!",
    image_too_many_scans:
      "A JPEG kép szokatlan módon, több mint %{max_scans} részletben tárolja a képet. Nyisd meg egy képszerkesztőben, mentsd el újra, majd töltsd fel ismét!",
    insufficient_storage:
      "A szerveren nincs elég szabad hely, ezért a feltöltés nem sikerült. Szólj a rendszergazdának!",
  } satisfies Record<ErrorCode, string>,
} as const satisfies MessageTree

/**
 * A nested message tree: every leaf is a polyglot phrase.
 *
 * Declared as an interface so it can refer to itself.
 */
export interface MessageTree {
  readonly [key: string]: string | MessageTree
}

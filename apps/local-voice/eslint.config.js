import i18next from "eslint-plugin-i18next";
import tsParser from "@typescript-eslint/parser";

export default [
  {
    // Vendorte Pakete (scripts/vendor-wai-*.py) werden nur im Quell-Repo
    // geändert und dort geprüft; Befunde gehen als Issue an wai-sdk.
    ignores: ["src/vendor/**"],
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaFeatures: {
          jsx: true,
        },
      },
    },
    plugins: {
      i18next,
    },
    rules: {
      // Never hand-roll a <select>.
      //
      // This is not a matter of taste and not "one of several elements to
      // avoid": a native <select> renders its OPTION LIST through the
      // operating system. No stylesheet reaches it. On this dark app that
      // means a white, unreadable popup — and no amount of classes on the
      // element itself changes that. <input> and <textarea> are different:
      // they are fully themable, and the handful of raw ones in this
      // codebase carry their own styling on purpose.
      //
      // Added 21.08.2026, after exactly this shipped as the speed picker in
      // the read-aloud transport bar. A rule catches it at lint time; a
      // review of a diff does not, because the diff looks fine.
      "no-restricted-syntax": [
        "error",
        {
          selector: "JSXOpeningElement[name.name='select']",
          message:
            "Use <Select> from components/ui/Select — a native <select> renders its option list through the OS and cannot be themed.",
        },
      ],
      // Catch text in JSX that should be translated
      "i18next/no-literal-string": [
        "error",
        {
          markupOnly: true, // Only check JSX content, not all strings
          ignoreAttribute: [
            "className",
            "style",
            "type",
            "id",
            "name",
            "key",
            "data-*",
            "aria-*",
          ], // Ignore common non-translatable attributes
        },
      ],
    },
  },
];

import parser from '@typescript-eslint/parser';
import plugin from '@typescript-eslint/eslint-plugin';

// Shared source libraries use the same baseline as the standalone Remote libs.
export default [{
  files: ['**/*.ts'],
  languageOptions: { parser, parserOptions: { sourceType: 'module' } },
  plugins: { '@typescript-eslint': plugin },
  rules: {
    ...plugin.configs.recommended.rules,
    '@typescript-eslint/no-non-null-assertion': 'off',
    '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_', varsIgnorePattern: '^_' }],
    // Micromark tokenizers carry their receiver into nested state functions.
    '@typescript-eslint/no-this-alias': ['error', { allowedNames: ['context'] }],
    'no-console': ['error', { allow: ['warn', 'error'] }],
    eqeqeq: ['error', 'always', { null: 'ignore' }],
  },
}, {
  files: ['**/*.test.ts'],
  rules: { '@typescript-eslint/no-explicit-any': 'off' },
}];

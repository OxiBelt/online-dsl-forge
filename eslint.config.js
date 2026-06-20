import tsPlugin from '@typescript-eslint/eslint-plugin'
import tsParser from '@typescript-eslint/parser'

const config = [
  {
    ignores: ['node_modules/**']
  },
  {
    files: ['devops/sources/**/*.ts', 'devops/tests/**/*.ts'],
    languageOptions: {
      parser: tsParser,
      sourceType: 'module'
    },
    plugins: {
      '@typescript-eslint': tsPlugin
    },
    rules: {
      ...tsPlugin.configs.recommended.rules,
      semi: ['error', 'never'],
      quotes: ['error', 'single'],
      '@typescript-eslint/consistent-type-definitions': ['error', 'type']
    }
  }
]

export default config

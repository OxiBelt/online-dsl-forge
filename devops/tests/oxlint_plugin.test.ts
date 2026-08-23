import * as Assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import * as Os from 'node:os'
import * as Path from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const repositoryRoot = Path.resolve(Path.dirname(fileURLToPath(import.meta.url)), '../..')
const oxlint = Path.join(repositoryRoot, 'node_modules', '.bin', 'oxlint')
const plugin = Path.join(repositoryRoot, 'devops', 'oxlint-plugin.mjs')

function lintFixture(source: string, rules: Record<string, 'error'>, fix = false) {
  const directory = mkdtempSync(Path.join(Os.tmpdir(), 'online-dsl-forge-oxlint-plugin-'))
  const sourcePath = Path.join(directory, 'fixture.ts')
  const configPath = Path.join(directory, '.oxlintrc.json')
  writeFileSync(sourcePath, source)
  writeFileSync(configPath, JSON.stringify({
    plugins: [],
    jsPlugins: [plugin],
    categories: { correctness: 'off' },
    rules
  }))

  const args = ['-c', configPath, sourcePath]
  if (fix) args.unshift('--fix')
  const result = spawnSync(oxlint, args, { encoding: 'utf8' })
  const output = `${result.stdout}${result.stderr}`
  const resultSource = readFileSync(sourcePath, 'utf8')
  rmSync(directory, { recursive: true, force: true })
  return { output, resultSource, status: result.status }
}

test('no-semicolons rejects only safely removable statement terminators', () => {
  const valid = lintFixture(`
for (let index = 0; index < 1; index += 1) {}
const first = () => {}; const second = first
const callable = () => {}
;(callable)()
class Example {
  get;
  value() {}
}
void second
`, { 'online-dsl-forge/no-semicolons': 'error' })
  Assert.equal(valid.status, 0, valid.output)

  const invalid = lintFixture('const value = 1;\nvoid value\n', {
    'online-dsl-forge/no-semicolons': 'error'
  })
  Assert.equal(invalid.status, 1, invalid.output)
  Assert.match(invalid.output, /online-dsl-forge\(no-semicolons\)/)
})

test('single-quotes rejects double quotes and simple templates', () => {
  const valid = lintFixture(`
const name = 'value'
const interpolated = \`value: \${name}\`
const tagged = String.raw\`value\`
void interpolated
void tagged
`, { 'online-dsl-forge/single-quotes': 'error' })
  Assert.equal(valid.status, 0, valid.output)

  const doubleQuoted = lintFixture('"use strict"\nconst value = "can\\\'t"\nvoid value\n', {
    'online-dsl-forge/single-quotes': 'error'
  })
  Assert.equal(doubleQuoted.status, 1, doubleQuoted.output)
  Assert.match(doubleQuoted.output, /online-dsl-forge\(single-quotes\)/)

  const simpleTemplate = lintFixture('const value = `value`\nvoid value\n', {
    'online-dsl-forge/single-quotes': 'error'
  })
  Assert.equal(simpleTemplate.status, 1, simpleTemplate.output)
})

test('custom compatibility rules remain diagnostic-only', () => {
  const source = 'const value = "value";\nvoid value\n'
  const result = lintFixture(source, {
    'online-dsl-forge/no-semicolons': 'error',
    'online-dsl-forge/single-quotes': 'error'
  }, true)
  Assert.equal(result.status, 1, result.output)
  Assert.equal(result.resultSource, source)
})

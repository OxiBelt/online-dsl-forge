import * as Fs from 'node:fs'
import * as Path from 'node:path'
import * as Process from 'node:process'
import { pathToFileURL } from 'node:url'
import * as Parsing from '@typescriptprime/parsing'
import * as Semver from 'semver'
import * as Toml from 'smol-toml'
import * as Zod from 'zod'

const PlaceholderVersion = '0.0.0'

type CliParameters = {
  Ref?: string
  WorkspacePath: string
  ManifestPath?: string
  LockfilePath: string
  PackageName?: string
  Packages?: string
  ReleasePublish?: boolean | string
}

export type PackageSpec = {
  packageName: string
  manifestPath: string
}

export type VersioningOptions = {
  ref?: string
  workspacePath: string
  manifestPath?: string
  lockfilePath: string
  packageName?: string
  packages?: PackageSpec[]
  releasePublish: boolean
}

type VersioningResult = {
  mode: 'check' | 'release'
  packageName: string
  version: string
}

type TomlRecord = Record<string, unknown>

type ResolvedPackageSpec = PackageSpec & {
  resolvedManifestPath: string
}

function isRecord(value: unknown): value is TomlRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function parseToml(content: string, filePath: string): TomlRecord {
  try {
    const parsed = Toml.parse(content)
    if (!isRecord(parsed)) {
      throw new Error('top-level TOML value is not an object')
    }
    return parsed
  } catch (error) {
    throw new Error(`${filePath} is not valid TOML: ${formatError(error)}`)
  }
}

function formatError(error: unknown): string {
  if (error instanceof Error) {
    return error.message
  }

  return String(error)
}

export function cleanVersionFromTagRef(ref: string): string {
  const prefix = 'refs/tags/'

  if (!ref.startsWith(prefix)) {
    throw new Error(`release ref must start with ${prefix}`)
  }

  const tag = ref.slice(prefix.length)
  const parsed = Semver.parse(tag)

  if (parsed === null) {
    throw new Error(`release tag must be valid SemVer: ${tag}`)
  }

  if (parsed.build.length > 0) {
    throw new Error(`Cargo release tags must not contain build metadata: ${tag}`)
  }

  return parsed.version
}

function resolveWorkspacePath(workspacePath: string): string {
  const resolved = Path.resolve(workspacePath)

  if (!Fs.existsSync(resolved) || !Fs.statSync(resolved).isDirectory()) {
    throw new Error(`workspace path is not a directory: ${workspacePath}`)
  }

  return resolved
}

function resolveWorkspaceFile(workspacePath: string, relativePath: string, label: string): string {
  if (Path.isAbsolute(relativePath)) {
    throw new Error(`${label} must be relative to the repository root: ${relativePath}`)
  }

  const resolved = Path.resolve(workspacePath, relativePath)
  const relative = Path.relative(workspacePath, resolved)

  if (relative === '' || relative.startsWith('..') || Path.isAbsolute(relative)) {
    throw new Error(`${label} must stay inside the repository root: ${relativePath}`)
  }

  if (!Fs.existsSync(resolved) || !Fs.statSync(resolved).isFile()) {
    throw new Error(`${label} does not exist: ${relativePath}`)
  }

  return resolved
}

function packageSpecsFromOptions(options: VersioningOptions): PackageSpec[] {
  if (options.packages !== undefined && options.packages.length > 0) {
    return assertUniquePackageSpecs(options.packages)
  }

  if (options.manifestPath !== undefined && options.packageName !== undefined) {
    return [{ packageName: options.packageName, manifestPath: options.manifestPath }]
  }

  throw new Error('versioning requires either --packages or both --manifest-path and --package-name')
}

function assertUniquePackageSpecs(packages: PackageSpec[]): PackageSpec[] {
  const names = new Set<string>()
  const manifestPaths = new Set<string>()

  for (const pkg of packages) {
    if (pkg.packageName.trim() === '') {
      throw new Error('package names must not be empty')
    }

    if (pkg.manifestPath.trim() === '') {
      throw new Error(`manifest path for ${pkg.packageName} must not be empty`)
    }

    if (names.has(pkg.packageName)) {
      throw new Error(`duplicate package name: ${pkg.packageName}`)
    }
    names.add(pkg.packageName)

    if (manifestPaths.has(pkg.manifestPath)) {
      throw new Error(`duplicate package manifest path: ${pkg.manifestPath}`)
    }
    manifestPaths.add(pkg.manifestPath)
  }

  return packages
}

function resolvePackageSpecs(workspacePath: string, packages: PackageSpec[]): ResolvedPackageSpec[] {
  return packages.map(pkg => ({
    ...pkg,
    resolvedManifestPath: resolveWorkspaceFile(
      workspacePath,
      pkg.manifestPath,
      `${pkg.packageName} manifest path`
    )
  }))
}

function parsePackageSpecs(value: string): PackageSpec[] {
  const packages = value.split(',').map(entry => {
    const separator = entry.indexOf('=')
    if (separator <= 0 || separator === entry.length - 1) {
      throw new Error(`package entries must use name=manifest-path: ${entry}`)
    }

    return {
      packageName: entry.slice(0, separator).trim(),
      manifestPath: entry.slice(separator + 1).trim()
    }
  })

  return assertUniquePackageSpecs(packages)
}

function displayPackageNames(packages: PackageSpec[]): string {
  return packages.map(pkg => pkg.packageName).join(',')
}

function packageTable(manifest: TomlRecord, manifestPath: string): TomlRecord {
  const packageData = manifest.package

  if (!isRecord(packageData)) {
    throw new Error(`${manifestPath} must contain a [package] table`)
  }

  return packageData
}

function lockPackageTable(lockfile: TomlRecord, lockfilePath: string, packageName: string): TomlRecord {
  const packages = lockfile.package

  if (!Array.isArray(packages)) {
    throw new Error(`${lockfilePath} must contain [[package]] entries`)
  }

  const matches = packages.filter((entry): entry is TomlRecord => {
    return isRecord(entry) && entry.name === packageName
  })

  if (matches.length !== 1) {
    throw new Error(`${lockfilePath} must contain exactly one ${packageName} package entry`)
  }

  return matches[0]
}

function assertManifestState(
  manifestPath: string,
  packageName: string,
  expectedVersion: string,
  expectedPublishFalse: boolean
): void {
  const manifest = parseToml(Fs.readFileSync(manifestPath, 'utf8'), manifestPath)
  const packageData = packageTable(manifest, manifestPath)

  if (packageData.name !== packageName) {
    throw new Error(`${manifestPath} package name must be ${packageName}`)
  }

  if (packageData.version !== expectedVersion) {
    throw new Error(`${manifestPath} package version must be ${expectedVersion}`)
  }

  if (expectedPublishFalse && packageData.publish !== false) {
    throw new Error(`${manifestPath} must keep publish = false in committed state`)
  }

  if (!expectedPublishFalse && packageData.publish === false) {
    throw new Error(`${manifestPath} must not keep publish = false in release state`)
  }
}

function assertLockfileState(lockfilePath: string, packageName: string, expectedVersion: string): void {
  const lockfile = parseToml(Fs.readFileSync(lockfilePath, 'utf8'), lockfilePath)
  const packageData = lockPackageTable(lockfile, lockfilePath, packageName)

  if (packageData.version !== expectedVersion) {
    throw new Error(`${lockfilePath} ${packageName} version must be ${expectedVersion}`)
  }
}

function dependencyTables(manifest: TomlRecord): TomlRecord[] {
  return ['dependencies', 'dev-dependencies', 'build-dependencies']
    .map(name => manifest[name])
    .filter(isRecord)
}

function dependencyVersion(dependency: unknown): string | undefined {
  if (typeof dependency === 'string') {
    return dependency
  }

  if (isRecord(dependency) && typeof dependency.version === 'string') {
    return dependency.version
  }

  return undefined
}

function assertLocalPackageDependencyVersions(
  manifestPath: string,
  packageNames: Set<string>,
  expectedVersion: string
): void {
  const manifest = parseToml(Fs.readFileSync(manifestPath, 'utf8'), manifestPath)

  for (const table of dependencyTables(manifest)) {
    for (const [dependencyName, dependency] of Object.entries(table)) {
      if (!packageNames.has(dependencyName)) {
        continue
      }

      if (dependencyVersion(dependency) !== expectedVersion) {
        throw new Error(`${manifestPath} dependency ${dependencyName} version must be ${expectedVersion}`)
      }
    }
  }
}

function packageSectionRange(content: string, manifestPath: string): [number, number] {
  const packageMatch = /^\[package\]\s*$/m.exec(content)

  if (packageMatch === null || packageMatch.index === undefined) {
    throw new Error(`${manifestPath} must contain a [package] table`)
  }

  const start = packageMatch.index
  const afterPackageHeader = start + packageMatch[0].length
  const nextTableMatch = /^\[.+\]\s*$/m.exec(content.slice(afterPackageHeader))
  const end = nextTableMatch === null ? content.length : afterPackageHeader + nextTableMatch.index

  return [start, end]
}

function replacePackageVersion(content: string, manifestPath: string, version: string): string {
  const [start, end] = packageSectionRange(content, manifestPath)
  const section = content.slice(start, end)
  const nextSection = section.replace(/^\s*version\s*=\s*"[^"]*"\s*$/m, `version = "${version}"`)

  if (nextSection === section) {
    throw new Error(`${manifestPath} [package] table must contain a version field`)
  }

  return `${content.slice(0, start)}${nextSection}${content.slice(end)}`
}

function removePackagePublishFalse(content: string, manifestPath: string): string {
  const [start, end] = packageSectionRange(content, manifestPath)
  const section = content.slice(start, end)
  const nextSection = section.replace(/^\s*publish\s*=\s*false\s*\r?\n/m, '')

  if (nextSection === section) {
    throw new Error(`${manifestPath} [package] table must contain publish = false before release`)
  }

  return `${content.slice(0, start)}${nextSection}${content.slice(end)}`
}

function replaceLocalPackageDependencyVersions(
  content: string,
  manifestPath: string,
  packageNames: Set<string>,
  version: string
): string {
  let nextContent = content

  for (const packageName of packageNames) {
    const escapedPackageName = escapeRegExp(packageName)
    const dependencyLine = new RegExp(
      `^(\\s*${escapedPackageName}\\s*=\\s*\\{[^\\n]*\\bversion\\s*=\\s*")([^"]*)("[^\\n]*\\}\\s*)$`,
      'm'
    )
    const hasDependency = new RegExp(`^\\s*${escapedPackageName}\\s*=`, 'm').test(nextContent)
    let replaced = false

    nextContent = nextContent.replace(dependencyLine, (_match, before: string, _oldVersion: string, after: string) => {
      replaced = true
      return `${before}${version}${after}`
    })

    if (hasDependency && !replaced) {
      throw new Error(
        `${manifestPath} dependency ${packageName} must use an inline table with a version field`
      )
    }
  }

  return nextContent
}

function lockPackageBlockRanges(content: string): Array<[number, number]> {
  const ranges: Array<[number, number]> = []
  const header = /^\[\[package\]\]\s*$/gm
  const starts: number[] = []
  let match: RegExpExecArray | null

  while ((match = header.exec(content)) !== null) {
    starts.push(match.index)
  }

  for (let index = 0; index < starts.length; index++) {
    ranges.push([starts[index], starts[index + 1] ?? content.length])
  }

  return ranges
}

function updateLockPackageVersion(
  content: string,
  lockfilePath: string,
  packageName: string,
  version: string
): string {
  const ranges = lockPackageBlockRanges(content)
  const matchingRanges = ranges.filter(([start, end]) => {
    const block = content.slice(start, end)
    return new RegExp(`^name\\s*=\\s*"${escapeRegExp(packageName)}"\\s*$`, 'm').test(block)
  })

  if (matchingRanges.length !== 1) {
    throw new Error(`${lockfilePath} must contain exactly one ${packageName} package block`)
  }

  const [start, end] = matchingRanges[0]
  const block = content.slice(start, end)
  const nextBlock = block.replace(/^\s*version\s*=\s*"[^"]*"\s*$/m, `version = "${version}"`)

  if (nextBlock === block) {
    throw new Error(`${lockfilePath} ${packageName} package block must contain a version field`)
  }

  return `${content.slice(0, start)}${nextBlock}${content.slice(end)}`
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

export function runVersioning(options: VersioningOptions): VersioningResult {
  const workspacePath = resolveWorkspacePath(options.workspacePath)
  const lockfilePath = resolveWorkspaceFile(workspacePath, options.lockfilePath, 'lockfile path')
  const packages = resolvePackageSpecs(workspacePath, packageSpecsFromOptions(options))
  const packageNames = new Set(packages.map(pkg => pkg.packageName))
  const packageName = displayPackageNames(packages)

  if (!options.releasePublish) {
    for (const pkg of packages) {
      assertManifestState(pkg.resolvedManifestPath, pkg.packageName, PlaceholderVersion, true)
      assertLockfileState(lockfilePath, pkg.packageName, PlaceholderVersion)
      assertLocalPackageDependencyVersions(pkg.resolvedManifestPath, packageNames, PlaceholderVersion)
    }

    return {
      mode: 'check',
      packageName,
      version: PlaceholderVersion
    }
  }

  if (options.ref === undefined) {
    throw new Error('release mode requires --ref')
  }

  const version = cleanVersionFromTagRef(options.ref)

  for (const pkg of packages) {
    assertManifestState(pkg.resolvedManifestPath, pkg.packageName, PlaceholderVersion, true)
    assertLockfileState(lockfilePath, pkg.packageName, PlaceholderVersion)
    assertLocalPackageDependencyVersions(pkg.resolvedManifestPath, packageNames, PlaceholderVersion)
  }

  for (const pkg of packages) {
    const nextManifest = replaceLocalPackageDependencyVersions(
      removePackagePublishFalse(
        replacePackageVersion(
          Fs.readFileSync(pkg.resolvedManifestPath, 'utf8'),
          pkg.resolvedManifestPath,
          version
        ),
        pkg.resolvedManifestPath
      ),
      pkg.resolvedManifestPath,
      packageNames,
      version
    )
    Fs.writeFileSync(pkg.resolvedManifestPath, nextManifest)
  }

  let nextLockfile = Fs.readFileSync(lockfilePath, 'utf8')
  for (const pkg of packages) {
    nextLockfile = updateLockPackageVersion(nextLockfile, lockfilePath, pkg.packageName, version)
  }
  Fs.writeFileSync(lockfilePath, nextLockfile)

  for (const pkg of packages) {
    assertManifestState(pkg.resolvedManifestPath, pkg.packageName, version, false)
    assertLockfileState(lockfilePath, pkg.packageName, version)
    assertLocalPackageDependencyVersions(pkg.resolvedManifestPath, packageNames, version)
  }

  return {
    mode: 'release',
    packageName,
    version
  }
}

function releasePublishEnabled(value: boolean | string | undefined): boolean {
  if (value === undefined) {
    return false
  }

  if (typeof value === 'boolean') {
    return value
  }

  return value === 'true'
}

async function parseCliParameters(): Promise<CliParameters> {
  const args = Parsing.FilterArgumentsForOptions(Process.argv)
  const parameters = (await Parsing.ParseArgumentsAndOptions<CliParameters>(args)).Options

  return Zod.strictObject({
    Ref: Zod.string().min(1).optional(),
    WorkspacePath: Zod.string().min(1),
    ManifestPath: Zod.string().min(1).optional(),
    LockfilePath: Zod.string().min(1),
    PackageName: Zod.string().min(1).optional(),
    Packages: Zod.string().min(1).optional(),
    ReleasePublish: Zod.union([Zod.boolean(), Zod.string()]).optional()
  }).parse(parameters)
}

async function main(): Promise<void> {
  const parameters = await parseCliParameters()
  const result = runVersioning({
    ref: parameters.Ref,
    workspacePath: parameters.WorkspacePath,
    manifestPath: parameters.ManifestPath,
    lockfilePath: parameters.LockfilePath,
    packageName: parameters.PackageName,
    packages: parameters.Packages === undefined ? undefined : parsePackageSpecs(parameters.Packages),
    releasePublish: releasePublishEnabled(parameters.ReleasePublish)
  })

  console.log(`${result.mode} versioning passed for ${result.packageName} ${result.version}`)
}

if (Process.argv[1] !== undefined && import.meta.url === pathToFileURL(Process.argv[1]).href) {
  main().catch(error => {
    console.error(formatError(error))
    Process.exit(1)
  })
}

/**
 * after-pack.mjs — electron-builder afterPack hook.
 *
 * Stamps the ulnclaw icon + identity onto the packed Windows ulnclaw.exe via
 * rcedit (delegated to set-exe-identity.mjs). This runs for EVERY packed build
 * — first install, `ulnclaw desktop`, the installer's --update rebuild, and a
 * dev's manual `npm run pack` — so the branded exe can never silently revert
 * to the stock "Electron" icon/name (the bug when the stamp lived only in
 * install.ps1, which the update path doesn't use).
 *
 * Windows-only: rcedit edits PE resources, irrelevant on macOS/Linux where the
 * app identity comes from the bundle Info.plist / desktop entry. Best-effort:
 * a stamp failure must never fail an otherwise-good build (worst case is the
 * stock icon, not a broken app), so we log and resolve rather than throw.
 *
 * electron-builder passes a context with:
 *   - electronPlatformName: 'win32' | 'darwin' | 'linux'
 *   - appOutDir:            the unpacked app directory for this target
 *   - packager.appInfo.productFilename: the exe basename (e.g. 'ulnclaw')
 */

import { execFile } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

import { stampExeIdentity } from './set-exe-identity.mjs'

function run(command, args) {
  return new Promise((resolve, reject) => {
    execFile(command, args, (error, stdout, stderr) => {
      if (error) {
        reject(new Error(`${command} ${args.join(' ')} failed: ${stderr?.trim() || stdout?.trim() || error.message}`))
        return
      }
      resolve({ stdout, stderr })
    })
  })
}

/**
 * Ad-hoc sign the packed macOS app bundle (no Apple Developer certificate in
 * CI). An UNSIGNED quarantined app opens with the dead-end "is damaged and
 * can't be opened — move to Trash" Gatekeeper verdict; an ad-hoc signed one
 * degrades to the "unidentified developer" dialog the user can approve via
 * right-click › Open or System Settings › Privacy & Security › Open Anyway.
 *
 * Signing order mirrors @electron/osx-sign: nested frameworks/helpers first
 * (each with the inherited entitlements), then the top-level bundle with the
 * main entitlements — signing the app last WITHOUT --deep so the nested
 * signatures survive.
 */
async function adhocSignMac(context) {
  const productName = context.packager?.appInfo?.productFilename || 'ulnclaw desktop'
  const appPath = path.join(context.appOutDir, `${productName}.app`)
  if (!fs.existsSync(appPath)) {
    console.warn(`[after-pack] skipping ad-hoc sign; missing bundle: ${appPath}`)
    return
  }

  const desktopRoot = path.resolve(import.meta.dirname, '..')
  const entitlements = path.join(desktopRoot, 'electron', 'entitlements.mac.plist')
  const entitlementsInherit = path.join(desktopRoot, 'electron', 'entitlements.mac.inherit.plist')
  const signNested = [
    '--force',
    '--deep',
    '--sign',
    '-',
    '--options',
    'runtime',
    '--entitlements',
    entitlementsInherit
  ]

  const frameworksDir = path.join(appPath, 'Contents', 'Frameworks')
  if (fs.existsSync(frameworksDir)) {
    for (const entry of fs.readdirSync(frameworksDir).sort()) {
      const nested = path.join(frameworksDir, entry)
      if (entry.endsWith('.framework') || entry.endsWith('.app')) {
        await run('codesign', [...signNested, nested])
      }
    }
  }

  await run('codesign', [
    '--force',
    '--sign',
    '-',
    '--options',
    'runtime',
    '--entitlements',
    entitlements,
    appPath
  ])
  await run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', appPath])
  console.log(`[after-pack] ad-hoc signed ${productName}.app (unidentified-developer flow instead of "damaged")`)
}

export default async function afterPack(context) {
  if (context.electronPlatformName === 'darwin') {
    // A signing failure must never brick an otherwise-good mac build: fall
    // back to the unsigned artifact (users xattr -cr, as before).
    try {
      await adhocSignMac(context)
    } catch (err) {
      console.warn(`[after-pack] ad-hoc codesign failed (${err.message}); shipping unsigned`)
    }
    return
  }

  if (context.electronPlatformName !== 'win32') {
    return
  }

  const productName = context.packager?.appInfo?.productFilename || 'ulnclaw'
  const exe = path.join(context.appOutDir, `${productName}.exe`)
  const desktopRoot = path.resolve(import.meta.dirname, '..')

  try {
    await stampExeIdentity(exe, desktopRoot)
  } catch (err) {
    // Never fail the build over a cosmetic stamp.
    console.warn(`[after-pack] exe identity stamp failed (${err.message}); ulnclaw.exe keeps the stock Electron icon`)
  }
}

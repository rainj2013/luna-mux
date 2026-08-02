import { describe, expect, it } from 'vitest'
import { parseSshConfig } from './ssh-config-import'

describe('parseSshConfig', () => {
  it('imports only concrete hosts and computes safe connection fields', () => {
    const entries = parseSshConfig(`
Host *
  User deploy
  Port 2200

Host jump
  HostName jump.example.test

Host prod
  HostName 10.0.0.8
  IdentityFile ~/.ssh/id_ed25519
  ProxyJump jump
  ProxyCommand ignored-command

Host dev-* !dev-old
  HostName dev.example.test

Match exec "exit 0"
  User should-not-apply
`, '/Users/tester')

    expect(entries).toEqual([
      { alias: 'jump', name: 'jump', host: 'jump.example.test', port: 2200, username: 'deploy', privateKeyPath: '', proxyJumpAlias: '' },
      { alias: 'prod', name: 'prod', host: '10.0.0.8', port: 2200, username: 'deploy', privateKeyPath: '/Users/tester/.ssh/id_ed25519', proxyJumpAlias: 'jump' }
    ])
  })
})

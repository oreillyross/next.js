import { runTests } from './utils'

it('should error when dynamic route is missing generateStaticParams', async () => {
  await runTests({
    isDev: true,
    dynamicPage: 'undefined',
    generateStaticParamsOpt: 'set noop',
    expectedErrMsg:
      'Page "/another/[slug]/page" is missing exported function "generateStaticParams()", which is required with "output: export" config.',
  })
})

it('should error when client component has generateStaticParams', async () => {
  await runTests({
    isDev: true,
    dynamicPage: 'undefined',
    generateStaticParamsOpt: 'set client',
    expectedErrMsg:
      'Page "/another/[slug]/page" cannot use both "use client" and export function "generateStaticParams()".',
  })
})

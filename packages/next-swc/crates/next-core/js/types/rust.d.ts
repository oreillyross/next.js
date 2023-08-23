// modules provided by rust

declare module 'PAGE' {
  import {
    NextPage,
    GetStaticPaths,
    GetServerSideProps,
    GetStaticProps,
  } from 'next'

  const Component: NextPage
  export default Component

  export const getStaticProps: GetStaticProps | undefined
  export const getStaticPaths: GetStaticPaths | undefined
  export const getServerSideProps: GetServerSideProps | undefined
}

declare module 'INNER' {
  export * from 'PAGE'
}

declare module 'CHUNK_GROUP' {
  const chunkGroup: import('types/next').ChunkGroup
  export default chunkGroup
}

declare module 'MIDDLEWARE_CHUNK_GROUP' {
  export { default } from 'CHUNK_GROUP'
}

declare module 'INNER_CLIENT_CHUNK_GROUP' {
  export { default } from 'CHUNK_GROUP'
}

declare module 'INNER_EDGE_CHUNK_GROUP' {
  export { default } from 'CHUNK_GROUP'
}

declare module 'ROUTE_CHUNK_GROUP' {
  export { default } from 'CHUNK_GROUP'
}

declare module 'MIDDLEWARE_CONFIG' {
  const matcher: string[]
  export default {
    matcher,
  }
}

declare module 'ENTRY' {
  // TODO: (wyattjoh) support other types of userland modules
  import type { AppRouteUserlandModule } from 'next/dist/server/future/route-modules/app-route/module'

  const module: AppRouteUserlandModule
  export = module
}

declare module 'BOOTSTRAP_CONFIG' {
  export const NAME: string
  export const PAGE: string
  export const PATHNAME: string
}

declare module 'APP_BOOTSTRAP' {
  const chunks: Array<string>
  export default chunks
}

declare module 'APP_ENTRY' {
  export const tree: any
  export const pathname: string
  // and more...
}

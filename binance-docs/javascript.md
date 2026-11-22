# JavaScript

The official JavaScript connector for Binance APIs, designed for backend applications running on
Node.js.

The connector is fully modular: each Binance product is published as a separate `npm` package (for
example `@binance/spot`), allowing you to install and use only the APIs you need. It supports REST
APIs, WebSocket APIs and WebSocket Streams and provides built-in request signing, connection
management and TypeScript definitions.

For source code, issues, and release notes, see the
[binance-connector-js](https://github.com/binance/binance-connector-js) repository on GitHub.

## Supported environments

- Node.js v22 or newer (LTS versions recommended)
- JavaScript and TypeScript backend applications

> This connector is intended for server-side usage only.  
> Browser environments are not supported at this time.

## Package structure

Each Binance product is published as an independent `npm` package under the `@binance` scope.

Commonly used packages include:

- `@binance/spot`
- `@binance/derivatives-trading-usds-futures`
- `@binance/derivatives-trading-coin-futures`
- `@binance/wallet`
- _(additional products follow the same structure)_

This modular approach keeps dependencies minimal and makes upgrades and integrations easier to
manage.

## Typical use cases

- Trading and execution services
- Real-time and historical market data ingestion
- Backend services managing accounts, balances, and orders
- Applications consuming WebSocket APIs or WebSocket Streams for low-latency updates

## Key features

- Support for Binance REST APIs, WebSocket APIs and WebSocket Streams
- Built-in request signing for authenticated endpoints
- WebSocket connection handling with automatic reconnection and session renewal
- First-class TypeScript definitions
- Support for both production and testnet environments
- Consistent API design across products

## Getting started

Install the package for the product you want to integrate with:

```bash
npm install @binance/spot
```

Create a client and send a request:

```ts
import { Spot } from "@binance/spot";

const configurationRestAPI = {
  apiKey: process.env.BINANCE_API_KEY,
  privateKey: process.env.BINANCE_PRIVATE_KEY,
};
const spotClient = new Spot({ configurationRestAPI });

const accountInfo = await spotClient.restAPI.getAccount();
```

The example above uses asymmetric key authentication. HMAC-based authentication using an API secret
is also supported.

Refer to each package’s documentation for product-specific REST, WebSocket API and WebSocket Streams
examples.

## Notes and best practices

- Store API keys securely using environment variables or a secrets manager
- Prefer WebSocket APIs or Streams for real-time data instead of polling REST endpoints
- Monitor rate limits and endpoint weights when building high-throughput services
- Use testnet environments when developing and validating integrations

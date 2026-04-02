# RFC 007: Plugin Protocol and Transport

Status: stub  
Owner: plugin/runtime lead  
Depends on: [RFC 001](./001-product-boundary.md)

## Purpose

Define the out-of-process plugin boundary for:

- tools
- signal sources
- channels
- post-turn analyzers
- policy hooks
- eval scorers

## Must Decide

1. Transport for v1
2. Capability negotiation
3. Permission and isolation model
4. Error and timeout semantics
5. Observability requirements

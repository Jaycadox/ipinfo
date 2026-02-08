# IPInfo

Locally query an IP address for information.

```sh
$ ipinfo 1.1.1.1
ASN: 13335,
Country Code: US,
Domain: cloudflare.com,
Name: CLOUDFLARENET,
Network: 1.1.1.0/24,
Organization: Cloudflare, Inc.
```

## Note
For this software to operate, a MMDB ip-to-asn [database](https://github.com/iplocate/ip-address-databases) (Creative Commons Attribution-ShareAlike 4.0 International License) is locally downloaded from IPLocate.io when the software is first ran. Future queries do not use the network.

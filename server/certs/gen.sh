#!/bin/bash

echo "1. Create Private Key for the Root CA"
[ ! -f rootCA.key ] && openssl genrsa -des3 -out rootCA.key 2048
echo "2. Create Certificate for the Root CA"
[ ! -f rootCA.pem ] && openssl req -x509 -new -nodes -key rootCA.key -sha256 -days 3650 -out rootCA.pem -config <(cat rootCA.cnf)

echo "3. Create Private Key the Server"
[ ! -f server.key ] && openssl req -new -sha256 -nodes -out server.csr -newkey rsa:2048 -keyout server.key -config <(cat rootCA.csr.cnf)
echo "4. Create Certificate for the Server"
[ ! -f server.crt ] && openssl x509 -req -in server.csr -CA rootCA.pem -CAkey rootCA.key -CAcreateserial -out server.crt -days 3650 -sha256 -extfile v3.ext

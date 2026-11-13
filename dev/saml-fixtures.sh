#!/bin/sh
# Regenerate the signed SAML responses used by the server's tests.
#
# The responses are signed by xmlsec1, an independent implementation of XML Signature, so
# the tests check Gantry's canonicalization and signature verification against something
# other than Gantry itself. Needs: openssl, xmlsec1. The output is committed; rerun this only
# to change the fixtures (the keys are throwaway and regenerated each time).
set -eu
out="$(cd "$(dirname "$0")/.." && pwd)/crates/gantry-server/tests/fixtures/saml"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$out"

for who in idp other; do
    openssl req -x509 -newkey rsa:2048 -nodes -keyout "$tmp/$who-key.pem" -out "$out/$who-cert.pem" \
        -days 36500 -subj "/CN=$who.example.org" 2>/dev/null
done
# The matching private key lets tests sign their own variations of a response.
cp "$tmp/idp-key.pem" "$out/idp-key.pem"

ACS="https://gantry.example.org/api/v1/auth/saml/11111111-1111-1111-1111-111111111111/acs"
SP="https://gantry.example.org/api/v1/auth/saml/11111111-1111-1111-1111-111111111111/metadata"
IDP="https://idp.example.org/entity"

signature() { # $1 = id of the signed element, $2 = signature method, $3 = digest method
    cat <<XML
<ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#">
      <ds:SignedInfo>
        <ds:CanonicalizationMethod Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/>
        <ds:SignatureMethod Algorithm="$2"/>
        <ds:Reference URI="#$1">
          <ds:Transforms>
            <ds:Transform Algorithm="http://www.w3.org/2000/09/xmldsig#enveloped-signature"/>
            <ds:Transform Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"><ec:InclusiveNamespaces xmlns:ec="http://www.w3.org/2001/10/xml-exc-c14n#" PrefixList="xs"/></ds:Transform>
          </ds:Transforms>
          <ds:DigestMethod Algorithm="$3"/>
          <ds:DigestValue></ds:DigestValue>
        </ds:Reference>
      </ds:SignedInfo>
      <ds:SignatureValue></ds:SignatureValue>
    </ds:Signature>
XML
}

assertion() { # $1 = signature block or empty, $2 = NameID content
    cat <<XML
<saml:Assertion xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" ID="_assert1" Version="2.0" IssueInstant="2026-06-01T12:00:00Z">
    <saml:Issuer>$IDP</saml:Issuer>
    $1
    <saml:Subject>
      <saml:NameID Format="urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress">$2</saml:NameID>
      <saml:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer">
        <saml:SubjectConfirmationData InResponseTo="_req1" NotOnOrAfter="2026-06-01T12:05:00Z" Recipient="$ACS"/>
      </saml:SubjectConfirmation>
    </saml:Subject>
    <saml:Conditions NotBefore="2026-06-01T11:59:00Z" NotOnOrAfter="2026-06-01T12:10:00Z">
      <saml:AudienceRestriction><saml:Audience>$SP</saml:Audience></saml:AudienceRestriction>
    </saml:Conditions>
    <saml:AuthnStatement AuthnInstant="2026-06-01T12:00:00Z" SessionIndex="_s1">
      <saml:AuthnContext><saml:AuthnContextClassRef>urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport</saml:AuthnContextClassRef></saml:AuthnContext>
    </saml:AuthnStatement>
    <saml:AttributeStatement>
      <saml:Attribute Name="displayName" NameFormat="urn:oasis:names:tc:SAML:2.0:attrname-format:basic">
        <saml:AttributeValue xsi:type="xs:string">Ada Lovelace &amp; Co. &lt;R&amp;D&gt;</saml:AttributeValue>
      </saml:Attribute>
      <saml:Attribute Name="mail" FriendlyName="email">
        <saml:AttributeValue xsi:type="xs:string">ada@example.org</saml:AttributeValue>
      </saml:Attribute>
      <saml:Attribute Name="groups">
        <saml:AttributeValue xsi:type="xs:string">engineering</saml:AttributeValue>
        <saml:AttributeValue xsi:type="xs:string">ops</saml:AttributeValue>
      </saml:Attribute>
    </saml:AttributeStatement>
  </saml:Assertion>
XML
}

response() { # $1 = signature block for the response or empty, $2 = assertion
    cat <<XML
<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="_resp1" Version="2.0" IssueInstant="2026-06-01T12:00:00Z" Destination="$ACS" InResponseTo="_req1">
  <saml:Issuer>$IDP</saml:Issuer>
  $1
  <samlp:Status>
    <samlp:StatusCode Value="urn:oasis:names:tc:SAML:2.0:status:Success"/>
  </samlp:Status>
  $2
</samlp:Response>
XML
}

RSA256="http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"
SHA256="http://www.w3.org/2001/04/xmlenc#sha256"
sign() { # $1 = key name, $2 = element whose ID attribute is referenced, $3 = input, $4 = output
    xmlsec1 --sign --privkey-pem "$tmp/$1-key.pem" --id-attr:ID "$2" --output "$out/$4" "$3"
}
A="urn:oasis:names:tc:SAML:2.0:assertion:Assertion"
R="urn:oasis:names:tc:SAML:2.0:protocol:Response"

response "" "$(assertion "$(signature _assert1 $RSA256 $SHA256)" ada@example.org)" > "$tmp/t.xml"
sign idp "$A" "$tmp/t.xml" assertion-signed.xml
sign other "$A" "$tmp/t.xml" assertion-signed-by-other-key.xml

response "$(signature _resp1 $RSA256 $SHA256)" "$(assertion "" ada@example.org)" > "$tmp/t.xml"
sign idp "$R" "$tmp/t.xml" response-signed.xml

response "" "$(assertion "$(signature _assert1 http://www.w3.org/2001/04/xmldsig-more#rsa-sha512 http://www.w3.org/2001/04/xmlenc#sha512)" ada@example.org)" > "$tmp/t.xml"
sign idp "$A" "$tmp/t.xml" assertion-signed-sha512.xml

response "" "$(assertion "$(signature _assert1 http://www.w3.org/2000/09/xmldsig#rsa-sha1 http://www.w3.org/2000/09/xmldsig#sha1)" ada@example.org)" > "$tmp/t.xml"
sign idp "$A" "$tmp/t.xml" assertion-signed-sha1.xml

# A comment inside the name is ignored by the signature. The whole text must still be read.
response "" "$(assertion "$(signature _assert1 $RSA256 $SHA256)" 'ada@example.org<!-- -->.attacker.test')" > "$tmp/t.xml"
sign idp "$A" "$tmp/t.xml" assertion-signed-comment-in-name.xml

# Every fixture must verify with xmlsec1 itself before it is trusted as a reference.
for f in assertion-signed assertion-signed-sha512 assertion-signed-sha1 assertion-signed-comment-in-name; do
    xmlsec1 --verify --pubkey-cert-pem "$out/idp-cert.pem" --id-attr:ID "$A" "$out/$f.xml" >/dev/null 2>&1 || { echo "self-check failed: $f" >&2; exit 1; }
done
xmlsec1 --verify --pubkey-cert-pem "$out/idp-cert.pem" --id-attr:ID "$R" "$out/response-signed.xml" >/dev/null 2>&1 || { echo "self-check failed: response-signed" >&2; exit 1; }
echo "wrote $(ls "$out" | wc -l) files to $out"

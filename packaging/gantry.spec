# Gantry: project management, issue tracking, service desk, knowledge base and wiki.
#
# One source package builds two groups of binary packages.
#
#   Core (dnf install gantry pulls all of them and every dependency):
#     gantry          server, command-line tool, service units, setup and backup scripts
#     gantry-web      the web application (static files)
#     gantry-manual   the web manual (static files)
#     gantry-mcp      stdio bridge for AI clients that cannot speak HTTP to /mcp
#
#   Optional add-ons (install only what you want):
#     gantry-addon-nasa         NASA project management and systems engineering content
#     gantry-addon-pgvector     approximate vector index in PostgreSQL (pgvector)
#     gantry-addon-valkey       counters and short-lived state in Valkey
#     gantry-addon-opensearch   full-text search in OpenSearch (Podman container)
#     gantry-addon-objectstore  attachments in an S3-compatible store (Podman container)
#
# Build:   packaging/build-rpms.sh            (see README.md, "Building the RPMs")
# Options: --with prebuilt   package target/release, web/dist and docs/manual/_site as found
#                            in the source tree instead of compiling (used by CI caches)
#          --with offline    build with vendored crates and an npm cache, no network

%bcond_with prebuilt
%bcond_with offline

# The release profile strips symbols; there is nothing left for a debuginfo package.
%global debug_package %{nil}
%global addon_requires Requires: gantry >= %{version}

Name:           gantry
Version:        0.2.0
Release:        1%{?dist}
Summary:        Project tracking, service desk and wiki for teams and AI agents
License:        Apache-2.0
URL:            https://example.org/gantry
Source0:        %{name}-%{version}.tar.gz

ExclusiveArch:  x86_64 aarch64
BuildRequires:  systemd-rpm-macros
%if %{without prebuilt}
BuildRequires:  cargo >= 1.82
BuildRequires:  rust >= 1.82
BuildRequires:  gcc
BuildRequires:  nodejs >= 20
BuildRequires:  npm
%endif

Requires:       %{name}-web = %{version}-%{release}
Requires:       %{name}-manual = %{version}-%{release}
# Developed and tested on PostgreSQL 16: the default on Enterprise Linux 10, and the
# postgresql:16 module stream on Enterprise Linux 9 (packaging/install.sh enables it).
Requires:       postgresql-server >= 16
Requires:       postgresql-contrib >= 16
Requires:       nginx
Requires:       openssl
Requires:       curl
Requires:       policycoreutils
Requires:       util-linux
Recommends:     %{name}-mcp = %{version}-%{release}
Recommends:     firewalld
Requires(pre):  shadow-utils
%{?systemd_requires}

%description
Gantry is a self-hosted application for planning and tracking work: projects
with boards and configurable workflows, a service desk with a customer
portal and service-level targets, and a wiki with simple and advanced
editors. It serves teams of people and AI agents through one authenticated
API, a web application and a Model Context Protocol endpoint.

This package holds the server and its command-line tool. It runs behind
nginx and stores its data in PostgreSQL. After installing, run gantry-setup
once.

%package web
Summary:        Gantry web application
BuildArch:      noarch
%description web
The browser application for Gantry, served by the gantry server.

%package manual
Summary:        Gantry web manual
BuildArch:      noarch
%description manual
User, application administrator and system administrator guides for Gantry,
served by the gantry server under /manual/ and readable from disk.

%package mcp
Summary:        Gantry Model Context Protocol bridge for stdio clients
%description mcp
gantry-mcp connects AI clients that start a local program and talk over
standard input and output to the /mcp endpoint of a Gantry server. It can be
installed on a workstation without the server.

%package addon-nasa
Summary:        Gantry add-on: NASA project management and systems engineering
BuildArch:      noarch
%{addon_requires}
%description addon-nasa
Work item types, workflows, a flight-project template and document templates
that follow NPR 7120.5 life-cycle phases and reviews and the NPR 7123.1
systems engineering processes: requirements, verification, reviews, review
item discrepancies, key decision points, waivers and controlled products.

%package addon-pgvector
Summary:        Gantry add-on: approximate vector search with pgvector
BuildArch:      noarch
%{addon_requires}
Requires:       pgvector
%description addon-pgvector
Switches semantic search from exact comparison to an HNSW index provided by
the pgvector extension. Worth installing once there are several hundred
thousand indexed passages.

%package addon-valkey
Summary:        Gantry add-on: Valkey for counters and short-lived state
BuildArch:      noarch
%{addon_requires}
Requires:       valkey
%description addon-valkey
Moves rate-limit counters and short-lived state from PostgreSQL to Valkey,
which takes that write load off the database on busy installations and is
shared by all application nodes.

%package addon-opensearch
Summary:        Gantry add-on: OpenSearch for full-text search
BuildArch:      noarch
%{addon_requires}
Requires:       podman
%description addon-opensearch
Runs a single OpenSearch node in a Podman container managed by systemd and
switches full-text search to it. Intended for large installations;
PostgreSQL search remains the default and needs nothing extra.

%package addon-objectstore
Summary:        Gantry add-on: S3-compatible object storage for attachments
BuildArch:      noarch
%{addon_requires}
Requires:       podman
Requires:       openssl
%description addon-objectstore
Runs SeaweedFS in a Podman container managed by systemd and stores
attachments in it through the S3 interface, so several application nodes can
share one attachment store. Run gantry-objectstore-setup after installing.

%prep
%autosetup -n %{name}-%{version}

%build
%if %{without prebuilt}
# The date shown under About for each component: the changelog date when rpm provides
# one (reproducible builds), otherwise the day of the build.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(date +%%s)}"
export GANTRY_BUILD_DATE="$(date -u -d "@$SOURCE_DATE_EPOCH" +%%Y-%%m-%%d)"
cargo build --release --locked %{?with_offline:--offline}
(cd web && npm ci --no-audit --no-fund %{?with_offline:--offline} && npm run build && npm run manual)
%endif
test -x target/release/gantry
test -x target/release/gantry-mcp
test -f web/dist/index.html
test -f docs/manual/_site/index.html

%check
%if %{without prebuilt}
# Unit tests only: integration, security and regression suites need a PostgreSQL
# server and run from the source tree (see README.md, "Tests").
cargo test --release --locked %{?with_offline:--offline} --lib --bins
(cd web && npm test)
%endif
target/release/gantry --version
target/release/gantry config generate-key >/dev/null
# The script that writes and upgrades the nginx site. Skips itself on a build machine
# without nginx.
packaging/tests/nginx-site.sh

%install
f=packaging/files
install -Dpm 0755 target/release/gantry       %{buildroot}%{_bindir}/gantry
install -Dpm 0755 target/release/gantry-mcp   %{buildroot}%{_bindir}/gantry-mcp
install -Dpm 0755 $f/gantry-setup             %{buildroot}%{_sbindir}/gantry-setup
install -Dpm 0755 $f/gantry-backup            %{buildroot}%{_sbindir}/gantry-backup
install -Dpm 0755 $f/addon-apply              %{buildroot}%{_libexecdir}/gantry/addon-apply
install -Dpm 0755 $f/nginx-site               %{buildroot}%{_libexecdir}/gantry/nginx-site
install -Dpm 0644 $f/gantry.service           %{buildroot}%{_unitdir}/gantry.service
install -Dpm 0644 $f/gantry-worker.service    %{buildroot}%{_unitdir}/gantry-worker.service
install -Dpm 0644 $f/gantry-backup.service    %{buildroot}%{_unitdir}/gantry-backup.service
install -Dpm 0644 $f/gantry-backup.timer      %{buildroot}%{_unitdir}/gantry-backup.timer
install -Dpm 0644 $f/gantry.sysusers          %{buildroot}%{_sysusersdir}/gantry.conf
install -Dpm 0644 $f/gantry.tmpfiles          %{buildroot}%{_tmpfilesdir}/gantry.conf
install -Dpm 0640 $f/gantry.toml              %{buildroot}%{_sysconfdir}/gantry/gantry.toml
install -Dpm 0644 $f/nginx-gantry.conf.in     %{buildroot}%{_datadir}/gantry/nginx/gantry.conf.in
install -d -m 0750 %{buildroot}%{_sysconfdir}/gantry/conf.d
install -d -m 0750 %{buildroot}%{_sharedstatedir}/gantry
install -d -m 0750 %{buildroot}%{_sharedstatedir}/gantry/blobs
install -d -m 0700 %{buildroot}%{_localstatedir}/backups/gantry
install -d         %{buildroot}%{_datadir}/gantry/addons

# Static content.
install -d %{buildroot}%{_datadir}/gantry/web %{buildroot}%{_datadir}/gantry/manual
cp -a web/dist/.           %{buildroot}%{_datadir}/gantry/web/
cp -a docs/manual/_site/.  %{buildroot}%{_datadir}/gantry/manual/

# Add-on: NASA content pack.
install -d %{buildroot}%{_datadir}/gantry/addons/nasa
cp -a addons/nasa/. %{buildroot}%{_datadir}/gantry/addons/nasa/

# Add-ons that switch a storage back end: one configuration drop-in each, and for
# services on this machine a unit drop-in so that they start with Gantry.
a=$f/addons
install -Dpm 0640 $a/50-pgvector.toml             %{buildroot}%{_sysconfdir}/gantry/conf.d/50-pgvector.toml
install -Dpm 0640 $a/50-valkey.toml               %{buildroot}%{_sysconfdir}/gantry/conf.d/50-valkey.toml
install -Dpm 0644 $a/valkey.dropin.conf           %{buildroot}%{_unitdir}/gantry.service.d/50-valkey.conf
install -Dpm 0640 $a/50-opensearch.toml           %{buildroot}%{_sysconfdir}/gantry/conf.d/50-opensearch.toml
install -Dpm 0644 $a/opensearch.dropin.conf       %{buildroot}%{_unitdir}/gantry.service.d/50-opensearch.conf
install -Dpm 0644 $a/gantry-opensearch.container  %{buildroot}%{_datadir}/containers/systemd/gantry-opensearch.container
install -Dpm 0644 $a/objectstore.dropin.conf      %{buildroot}%{_unitdir}/gantry.service.d/50-objectstore.conf
install -Dpm 0644 $a/gantry-objectstore.container %{buildroot}%{_datadir}/containers/systemd/gantry-objectstore.container
install -Dpm 0755 $a/gantry-objectstore-setup     %{buildroot}%{_sbindir}/gantry-objectstore-setup
for u in valkey opensearch objectstore; do install -Dpm 0644 $a/$u.dropin.conf %{buildroot}%{_unitdir}/gantry-worker.service.d/50-$u.conf; done

install -Dpm 0644 LICENSE   %{buildroot}%{_licensedir}/%{name}/LICENSE
install -Dpm 0644 README.md %{buildroot}%{_docdir}/%{name}/README.md

%dnl ---- scriptlets: core ----
%pre
# The service account. Created here as well as declared in sysusers.d so that it exists
# before the files it owns are unpacked, on every supported release.
getent group gantry >/dev/null || groupadd -r gantry
getent passwd gantry >/dev/null || useradd -r -g gantry -d %{_sharedstatedir}/gantry -s /sbin/nologin -c "Gantry service" gantry
exit 0

%post
%tmpfiles_create %{_tmpfilesdir}/gantry.conf
%systemd_post gantry.service gantry-worker.service gantry-backup.timer
if [ $1 -eq 1 ]; then
    cat <<'MSG'

  Gantry is installed. Finish with one command, as root:

      gantry-setup --hostname gantry.example.org --admin-email you@example.org

  It prepares PostgreSQL, creates the secret key and the first administrator,
  configures nginx with TLS and starts the service.

MSG
fi

%preun
%systemd_preun gantry.service gantry-worker.service gantry-backup.timer

%postun
# On upgrade the running services are restarted; gantry.service migrates the schema
# before it starts serving (ExecStartPre=gantry migrate).
%systemd_postun_with_restart gantry.service gantry-worker.service

%posttrans
# The nginx site that gantry-setup wrote is brought up to this version's template (each
# release may add locations: 0.2.0 added the WebSocket endpoint and the page shown while
# the application is unavailable). A site the administrator has taken over is left alone.
%{_libexecdir}/gantry/nginx-site refresh || :

%dnl The web application and manual are read from disk on each request; nothing to restart.
%dnl
%dnl ---- scriptlets: add-ons ----
%dnl Content packs take effect in the database, storage add-ons on the next start.
%dnl The addon-apply helper does either at once when the server is already set up and
%dnl running; otherwise gantry-setup or the next service start does.

%posttrans addon-nasa
%{_libexecdir}/gantry/addon-apply pack nasa
exit 0

%post addon-pgvector
if systemctl is-active --quiet postgresql.service; then
    runuser -u postgres -- psql -Xq -d gantry -c 'CREATE EXTENSION IF NOT EXISTS vector' >/dev/null 2>&1 || \
        echo "gantry: enable pgvector in the database with: gantry-setup" >&2
fi
exit 0

%posttrans addon-pgvector
%{_libexecdir}/gantry/addon-apply restart
exit 0

%postun addon-pgvector
[ $1 -eq 0 ] && systemctl try-restart gantry.service gantry-worker.service >/dev/null 2>&1
exit 0

%posttrans addon-valkey
systemctl daemon-reload >/dev/null 2>&1 || :
systemctl enable --now valkey.service >/dev/null 2>&1 || :
%{_libexecdir}/gantry/addon-apply restart
exit 0

%postun addon-valkey
systemctl daemon-reload >/dev/null 2>&1 || :
[ $1 -eq 0 ] && systemctl try-restart gantry.service gantry-worker.service >/dev/null 2>&1
exit 0

%posttrans addon-opensearch
systemctl daemon-reload >/dev/null 2>&1 || :
if [ -s %{_sysconfdir}/gantry/secret.key ]; then
    systemctl start gantry-opensearch.service >/dev/null 2>&1 || echo "gantry: start OpenSearch with: systemctl start gantry-opensearch" >&2
    cat <<'MSG'

  OpenSearch is starting (the first start downloads the container image).
  When "systemctl is-active gantry-opensearch" reports active, switch Gantry over:

      systemctl restart gantry
      runuser -u gantry -- gantry reindex

MSG
fi
exit 0

%preun addon-opensearch
[ $1 -eq 0 ] && systemctl stop gantry-opensearch.service >/dev/null 2>&1
exit 0

%postun addon-opensearch
systemctl daemon-reload >/dev/null 2>&1 || :
if [ $1 -eq 0 ]; then
    systemctl try-restart gantry.service gantry-worker.service >/dev/null 2>&1
    echo "gantry: search is back on PostgreSQL; rebuild its index with: runuser -u gantry -- gantry reindex" >&2
fi
exit 0

%posttrans addon-objectstore
systemctl daemon-reload >/dev/null 2>&1 || :
echo "gantry: finish with: gantry-objectstore-setup" >&2
exit 0

%preun addon-objectstore
[ $1 -eq 0 ] && systemctl stop gantry-objectstore.service >/dev/null 2>&1
exit 0

%postun addon-objectstore
systemctl daemon-reload >/dev/null 2>&1 || :
exit 0

%dnl ---- files ----
%files
%license %{_licensedir}/%{name}/LICENSE
%doc %{_docdir}/%{name}/README.md
%{_bindir}/gantry
%{_sbindir}/gantry-setup
%{_sbindir}/gantry-backup
%{_libexecdir}/gantry/
%{_unitdir}/gantry.service
%{_unitdir}/gantry-worker.service
%{_unitdir}/gantry-backup.service
%{_unitdir}/gantry-backup.timer
%{_sysusersdir}/gantry.conf
%{_tmpfilesdir}/gantry.conf
%dir %{_datadir}/gantry
%dir %{_datadir}/gantry/addons
%{_datadir}/gantry/nginx/
%dir %attr(0750,root,gantry) %{_sysconfdir}/gantry
%dir %attr(0750,root,gantry) %{_sysconfdir}/gantry/conf.d
%config(noreplace) %attr(0640,root,gantry) %{_sysconfdir}/gantry/gantry.toml
%ghost %attr(0640,root,gantry) %{_sysconfdir}/gantry/secret.key
# Written by gantry-setup; owned here so that removing the package removes the site.
%ghost %config(noreplace) %{_sysconfdir}/nginx/conf.d/gantry.conf
%dir %attr(0750,gantry,gantry) %{_sharedstatedir}/gantry
%dir %attr(0750,gantry,gantry) %{_sharedstatedir}/gantry/blobs
%dir %attr(0700,root,root) %{_localstatedir}/backups/gantry

%files web
%{_datadir}/gantry/web/

%files manual
%{_datadir}/gantry/manual/

%files mcp
%license %{_licensedir}/%{name}/LICENSE
%{_bindir}/gantry-mcp

%files addon-nasa
%{_datadir}/gantry/addons/nasa/

%files addon-pgvector
%config(noreplace) %attr(0640,root,gantry) %{_sysconfdir}/gantry/conf.d/50-pgvector.toml

%files addon-valkey
%config(noreplace) %attr(0640,root,gantry) %{_sysconfdir}/gantry/conf.d/50-valkey.toml
%dir %{_unitdir}/gantry.service.d
%dir %{_unitdir}/gantry-worker.service.d
%{_unitdir}/gantry.service.d/50-valkey.conf
%{_unitdir}/gantry-worker.service.d/50-valkey.conf

%files addon-opensearch
%config(noreplace) %attr(0640,root,gantry) %{_sysconfdir}/gantry/conf.d/50-opensearch.toml
%dir %{_unitdir}/gantry.service.d
%dir %{_unitdir}/gantry-worker.service.d
%{_unitdir}/gantry.service.d/50-opensearch.conf
%{_unitdir}/gantry-worker.service.d/50-opensearch.conf
%{_datadir}/containers/systemd/gantry-opensearch.container

%files addon-objectstore
%{_sbindir}/gantry-objectstore-setup
%dir %{_unitdir}/gantry.service.d
%dir %{_unitdir}/gantry-worker.service.d
%{_unitdir}/gantry.service.d/50-objectstore.conf
%{_unitdir}/gantry-worker.service.d/50-objectstore.conf
%{_datadir}/containers/systemd/gantry-objectstore.container
%ghost %attr(0640,root,gantry) %{_sysconfdir}/gantry/conf.d/50-objectstore.toml
%ghost %dir %attr(0750,root,root) %{_sysconfdir}/gantry/objectstore

%changelog
* Wed Nov 11 2026 Gantry maintainers <maintainers@example.org> - 0.2.0-1
- Notifications by email with personal and workspace choices; alert rules for
  application and system administrators, also to Slack and Teams.
- A discussion on every work item and request; mentions, links, attachments.
- Live updates and editing at the same time over a WebSocket (/api/v1/ws). The
  nginx site written by gantry-setup is brought up to date by this package;
  a site maintained by hand needs the new location (see the upgrade notes).
- SAML 2.0 sign-in. Rules that limit what AI agent accounts may do; agents that
  existed before the upgrade keep what their roles allow until rules are set.
- Hierarchy levels above projects, graph, reports, search by criteria, Gantt
  charts edited in place, visual workflow editor, history of every change.
- Dashboards and custom visualizations in a sealed frame; diagram editor;
  comparison before publishing a page.
- Profiles, avatars, status, teams, positions and an org chart.
- About pages (versions, statements, licenses), pages for every error, light and
  dark manual, accessibility checks.
- The API address /api/v1/catalogue is now /api/v1/catalog, and the built-in
  status key "cancelled" is now "canceled" (items are migrated).

* Tue Nov 10 2026 Gantry maintainers <maintainers@example.org> - 0.1.0-1
- First release: projects, boards and workflows, service desk, wiki, search,
  integrations, MCP endpoint, administration console, NASA add-on.

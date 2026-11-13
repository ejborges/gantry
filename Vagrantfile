# Development machine for Gantry: an Enterprise Linux guest on VMware with everything
# needed to build, test and package.
#
#   vagrant plugin install vagrant-vmware-desktop     (once; also needs the Vagrant VMware Utility)
#   vagrant up --provider vmware_desktop
#   vagrant ssh
#   cd /vagrant && dev/run-local.sh                   -> http://localhost:8600 on your computer
#
# Choose another box with GANTRY_BOX (any EL 9 or 10 box that supports vmware_desktop):
#   GANTRY_BOX=almalinux/10 vagrant up --provider vmware_desktop
# AlmaLinux publishes its boxes on its own server; if Vagrant cannot find them, set
#   VAGRANT_SERVER_URL=https://vagrant.almalinux.org       (PowerShell: $env:VAGRANT_SERVER_URL = "...")
#
# The source tree is copied into the guest with rsync, which works on every host and keeps
# builds on the guest's own disk. After editing on the host run `vagrant rsync`, or leave
# `vagrant rsync-auto` running. Work done inside the guest stays in the guest.

Vagrant.configure("2") do |config|
  config.vm.box = ENV.fetch("GANTRY_BOX", "almalinux/10")
  config.vm.hostname = "gantry-dev"

  config.vm.provider "vmware_desktop" do |v|
    v.vmx["displayName"] = "gantry-dev"
    v.vmx["numvcpus"] = ENV.fetch("GANTRY_VM_CPUS", "4")
    v.vmx["memsize"]  = ENV.fetch("GANTRY_VM_MEMORY_MB", "8192")
    v.gui = true
  end

  # 8600: Gantry started with dev/run-local.sh.   5173: Vite dev server (npm run dev).
  # 8443: nginx, when you install the RPMs in the guest to test packaging.
  # If a port on your computer is taken, choose another with GANTRY_HOST_PORT_APP,
  # GANTRY_HOST_PORT_VITE or GANTRY_HOST_PORT_HTTPS. Without that, Vagrant picks a free port
  # itself and prints it during `vagrant up` (`vagrant port` shows them later).
  {
    8600 => ENV.fetch("GANTRY_HOST_PORT_APP", "8600"),
    5173 => ENV.fetch("GANTRY_HOST_PORT_VITE", "5173"),
    443  => ENV.fetch("GANTRY_HOST_PORT_HTTPS", "8443"),
  }.each do |guest, host|
    config.vm.network "forwarded_port", guest: guest, host: Integer(host), host_ip: "127.0.0.1", auto_correct: true
  end

  config.vm.synced_folder ".", "/vagrant", type: "rsync",
    rsync__exclude: [".git/", ".vagrant/", "target/", "build/", "dist/", "web/node_modules/", "web/dist/", "web/test-results/", "docs/manual/_site/"],
    rsync__args: ["--archive", "--delete", "-z"]

  config.vm.provision "shell", path: "dev/provision.sh", privileged: true
end
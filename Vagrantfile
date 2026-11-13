# Development machine for Gantry: an Enterprise Linux guest on VMware with everything
# needed to build, test and package, and a desktop to work in.
#
#   vagrant plugin install vagrant-vmware-desktop     (once; also needs the Vagrant VMware Utility)
#   vagrant up --provider vmware_desktop
#   vagrant ssh
#   cd /vagrant && dev/run-local.sh
#
# Choose another box with GANTRY_BOX (any EL 9 or 10 box that supports vmware_desktop):
#   GANTRY_BOX=almalinux/10 vagrant up --provider vmware_desktop
# AlmaLinux publishes its boxes on its own server; if Vagrant cannot find them, set
#   VAGRANT_SERVER_URL=https://vagrant.almalinux.org       (PowerShell: $env:VAGRANT_SERVER_URL = "...")
#
# The desktop: the machine gets GNOME, Firefox and the VMware display tools, so its window
# can be any size. Sign in as "vagrant" with the password "vagrant" (GANTRY_VM_PASSWORD
# changes it). To keep it in your VMware library and start it without Vagrant, run
# dev/vm-open.ps1 once on Windows (Fusion on a Mac: File > Open, and choose the .vmx file that
# `vagrant up` names). GANTRY_VM_GUI=0 leaves the desktop out; GANTRY_VM_WINDOW=1 makes
# `vagrant up` open the VMware window too.
#
# Ports: inside the machine Gantry is always on 8600. On your computer Vagrant forwards
# 8600, 5173 and 8443, or other free ports when those are taken, and says which after every
# `vagrant up`. They are also written to .vagrant/gantry-ports.env (and .json) for scripts on
# your computer, and to /etc/gantry-dev/host-ports.env inside the machine. To ask for
# particular ports: GANTRY_HOST_PORT_APP, GANTRY_HOST_PORT_VITE, GANTRY_HOST_PORT_HTTPS.
#
# The source tree is copied into the guest with rsync, which works on every host and keeps
# builds on the guest's own disk. After editing on the host run `vagrant rsync`, or leave
# `vagrant rsync-auto` running. Work done inside the guest stays in the guest.

require "json"

# Finds the ports Vagrant actually forwarded (after moving any that were taken) and tells
# you, your computer's scripts and the machine itself.
module GantryVM
  NAMES = { 8600 => "APP", 5173 => "VITE", 443 => "HTTPS", 22 => "SSH" }.freeze

  def self.forwarded(machine)
    found = {}
    if machine.provider.capability?(:forwarded_ports)
      (machine.provider.capability(:forwarded_ports) || {}).each { |host, guest| found[guest.to_i] = host.to_i }
    end
    if found.empty? # The collision check corrects these in place during `vagrant up`.
      machine.config.vm.networks.each do |type, o|
        found[o[:guest].to_i] = o[:host].to_i if type == :forwarded_port && o[:guest] && o[:host]
      end
    end
    found
  rescue StandardError
    {}
  end

  def self.report(machine)
    ports = forwarded(machine)
    ip = +""
    machine.communicate.execute("hostname -I", error_check: false) { |kind, data| ip << data if kind == :stdout }
    ip = ip.split.first.to_s
    app = ports[8600]
    info = { "app" => app, "vite" => ports[5173], "https" => ports[443], "ssh" => ports[22], "guest_ip" => ip, "vmx" => machine.id.to_s }
    info["url"] = "http://localhost:#{app}" if app

    data = machine.env.local_data_path
    File.write(File.join(data, "gantry-ports.json"), JSON.pretty_generate(info) + "\n")
    host_env = NAMES.filter_map { |guest, name| "GANTRY_HOST_PORT_#{name}=#{ports[guest]}" if ports[guest] }
    host_env << "GANTRY_E2E_URL=http://localhost:#{app}" if app
    host_env << "GANTRY_GUEST_IP=#{ip}" unless ip.empty?
    File.write(File.join(data, "gantry-ports.env"), host_env.join("\n") + "\n")

    # Inside the machine: the same ports, without GANTRY_E2E_URL (tests there use 8600).
    guest_env = host_env.reject { |l| l.start_with?("GANTRY_E2E_URL=") }.join("\n")
    machine.communicate.sudo("install -d -m 0755 /etc/gantry-dev && printf '%s\\n' #{guest_env.split("\n").map { |l| "'#{l}'" }.join(' ')} > /etc/gantry-dev/host-ports.env", error_check: false)

    machine.ui.info("Gantry, from your computer:")
    machine.ui.info("  application  http://localhost:#{app}   (after dev/run-local.sh in the machine)") if app
    machine.ui.info("  Vite         http://localhost:#{ports[5173]}   (after npm run dev)") if ports[5173]
    machine.ui.info("  nginx        https://localhost:#{ports[443]}   (after installing the RPMs)") if ports[443]
    machine.ui.info("  or directly  http://#{ip}:8600") unless ip.empty?
    machine.ui.info("  These moved because the usual ports were taken; see `vagrant port`.") if app && app != 8600
    machine.ui.info("VMware machine: #{machine.id}") if machine.id
  rescue StandardError => e
    machine.ui.warn("Gantry: could not report the forwarded ports (#{e.message}); run `vagrant port`.")
  end
end

Vagrant.configure("2") do |config|
  config.vm.box = ENV.fetch("GANTRY_BOX", "almalinux/9")
  config.vm.hostname = "gantry-dev"
  desktop = ENV.fetch("GANTRY_VM_GUI", "1") == "1"

  config.vm.provider "vmware_desktop" do |v|
    v.vmx["displayName"] = "gantry-dev"
    v.vmx["numvcpus"] = ENV.fetch("GANTRY_VM_CPUS", "4")
    v.vmx["memsize"]  = ENV.fetch("GANTRY_VM_MEMORY_MB", "8192")
    v.gui = ENV.fetch("GANTRY_VM_WINDOW", "0") == "1"
    if desktop
      # Accelerated graphics and enough video memory for a large or high-resolution window;
      # the window size follows the VMware window (open-vm-tools-desktop, installed below).
      v.vmx["mks.enable3d"] = "TRUE"
      v.vmx["svga.autodetect"] = "TRUE"
      v.vmx["svga.graphicsMemoryKB"] = "2097152"
      v.vmx["svga.vramSize"] = "268435456"
      v.vmx["sound.present"] = "TRUE"
      v.vmx["sound.autodetect"] = "TRUE"
      v.vmx["usb.present"] = "TRUE"
      v.vmx["isolation.tools.copy.disable"] = "FALSE"
      v.vmx["isolation.tools.paste.disable"] = "FALSE"
    end
  end

  # 8600: Gantry started with dev/run-local.sh.   5173: Vite dev server (npm run dev).
  # 8443: nginx, when you install the RPMs in the guest to test packaging.
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

  config.vm.provision "shell", path: "dev/provision.sh", privileged: true,
    env: { "GANTRY_DESKTOP" => desktop ? "1" : "0", "GANTRY_VM_PASSWORD" => ENV.fetch("GANTRY_VM_PASSWORD", "vagrant") }

  config.trigger.after [:up, :reload, :resume] do |t|
    t.name = "Gantry: forwarded ports"
    t.ruby { |_env, machine| GantryVM.report(machine) }
  end
end

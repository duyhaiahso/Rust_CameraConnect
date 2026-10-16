use std::net::Ipv4Addr;

pub struct IpSettings {
    pub ip: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub gateway: Ipv4Addr,
}

pub struct IpValidationError {
    pub english: &'static str,
    pub vietnamese: &'static str,
}

impl IpValidationError {
    const fn new(english: &'static str, vietnamese: &'static str) -> Self {
        Self {
            english,
            vietnamese,
        }
    }
}

impl IpSettings {
    pub fn parse(
        ip: &str,
        mask: &str,
        gateway: &str,
        interface_ip: &str,
    ) -> Result<Self, IpValidationError> {
        let ip: Ipv4Addr = ip.trim().parse().map_err(|_| {
            IpValidationError::new("Invalid camera IP address", "IP camera không hợp lệ")
        })?;
        let mask: Ipv4Addr = mask.trim().parse().map_err(|_| {
            IpValidationError::new("Invalid subnet mask", "Subnet mask không hợp lệ")
        })?;
        let gateway: Ipv4Addr = gateway
            .trim()
            .parse()
            .map_err(|_| IpValidationError::new("Invalid gateway", "Gateway không hợp lệ"))?;
        let bits = u32::from(mask);
        if !(1..=30).contains(&bits.count_ones())
            || !(!bits).checked_add(1).is_some_and(u32::is_power_of_two)
        {
            return Err(IpValidationError::new(
                "Subnet mask must have contiguous one bits and host addresses",
                "Subnet mask phải có các bit 1 liên tiếp và còn địa chỉ host",
            ));
        }
        let ip_bits = u32::from(ip);
        let network = ip_bits & bits;
        let broadcast = network | !bits;
        if ip_bits == network
            || ip_bits == broadcast
            || ip.is_loopback()
            || ip.is_multicast()
            || ip.is_unspecified()
        {
            return Err(IpValidationError::new(
                "Camera IP must be a usable host address",
                "IP camera phải là địa chỉ host hợp lệ",
            ));
        }
        if let Ok(host) = interface_ip.parse::<Ipv4Addr>() {
            if host == ip {
                return Err(IpValidationError::new(
                    "Camera IP matches the computer network interface",
                    "IP camera trùng IP cổng mạng máy tính",
                ));
            }
            if (u32::from(host) & bits) != network {
                return Err(IpValidationError::new(
                    "Camera IP and host interface must be in the same subnet under the entered mask",
                    "IP camera và cổng mạng máy tính phải cùng subnet theo mask đã nhập",
                ));
            }
        }
        if !gateway.is_unspecified() {
            let gateway_bits = u32::from(gateway);
            if gateway == ip
                || (gateway_bits & bits) != network
                || gateway_bits == network
                || gateway_bits == broadcast
            {
                return Err(IpValidationError::new(
                    "Gateway must be in the same subnet and differ from camera IP",
                    "Gateway phải nằm trong cùng subnet và khác IP camera",
                ));
            }
        }
        Ok(Self { ip, mask, gateway })
    }
}

#[cfg(test)]
mod tests {
    use super::IpSettings;

    #[test]
    fn accepts_camera_on_host_subnet() {
        assert!(
            IpSettings::parse("192.168.1.20", "255.255.255.0", "0.0.0.0", "192.168.1.10").is_ok()
        );
    }

    #[test]
    fn rejects_wrong_subnet_and_noncontiguous_mask() {
        assert!(
            IpSettings::parse("192.168.2.20", "255.255.255.0", "0.0.0.0", "192.168.1.10").is_err()
        );
        assert!(
            IpSettings::parse("192.168.1.20", "255.0.255.0", "0.0.0.0", "192.168.1.10").is_err()
        );
    }
}

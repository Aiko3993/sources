use aidoku::{
    Result,
    alloc::{String, format},
    imports::net::Request,
};

pub const ACCOUNT_API: &str = "https://account-api.zaimanhua.com/v1/";
pub const SIGN_API: &str = "https://i.zaimanhua.com/lpi/v1/";
pub const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 10) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36";

pub fn md5_hex(input: &str) -> String {
    let digest = md5::compute(input.as_bytes());
    format!("{:x}", digest)
}

pub fn get_request(url: &str) -> Result<Request> {
    Ok(Request::get(url)?.header("User-Agent", USER_AGENT))
}

pub fn post_request(url: &str) -> Result<Request> {
    Ok(Request::post(url)?
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/x-www-form-urlencoded"))
}

pub fn auth_request(url: &str, token: &str) -> Result<Request> {
    Ok(Request::get(url)?
        .header("User-Agent", USER_AGENT)
        .header("Authorization", &format!("Bearer {}", token)))
}

/// Login to Zaimanhua and return JWT token
pub fn login(username: &str, password: &str) -> Result<Option<String>> {
    let password_hash = md5_hex(password);
    let url = format!("{}login/passwd", ACCOUNT_API);
    let body = format!("username={}&passwd={}", username, password_hash);
    
    let mut response = post_request(&url)?
        .body(body.as_bytes())
        .send()?;
    
    let json: serde_json::Value = response.get_json()?;
    
    // Check errno
    let errno = json.get("errno").and_then(|v| v.as_i64()).unwrap_or(-1);
    if errno != 0 {
        return Ok(None);
    }
    
    // Extract token: data.user.token
    if let Some(token) = json
        .get("data")
        .and_then(|d| d.get("user"))
        .and_then(|u| u.get("token"))
        .and_then(|t| t.as_str())
    {
        return Ok(Some(token.into()));
    }
    
    Ok(None)
}

/// Perform daily check-in (POST request required!)
pub fn check_in(token: &str) -> Result<bool> {
    let url = format!("{}task/sign_in", SIGN_API);
    let mut response = Request::post(&url)?
        .header("User-Agent", USER_AGENT)
        .header("Authorization", &format!("Bearer {}", token))
        .send()?;
    
    let json: serde_json::Value = response.get_json()?;
    let errno = json.get("errno").and_then(|v| v.as_i64()).unwrap_or(-1);
    Ok(errno == 0)
}


/// Get user info (for level, points, VIP status etc)
pub fn get_user_info(token: &str) -> Result<serde_json::Value> {
    let url = format!("{}userInfo/get", SIGN_API);
    let mut response = auth_request(&url, token)?.send()?;
    response.get_json()
}
